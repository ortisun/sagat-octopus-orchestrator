//! The state of one run and the reducer that folds journal events into it.
//!
//! [`RunState::apply`] is the only way state changes. It needs no workflow definition, so a run can
//! be rebuilt from its journal alone ([`RunState::replay`]) and the stored projection can be checked
//! against that rebuild. Every event is validated before anything is mutated: a rejected event
//! leaves the state exactly as it was.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use time::OffsetDateTime;

use super::error::ExecutionError;
use super::event::{
    ChildOutcome, CorrelationKey, FailureCode, JoinOutcome, ReconcileResult, ReviewOutcome,
    RunEvent, StopReason,
};
use super::instance::ExecutionInstanceId;
use crate::identity::IdentityError;
use crate::saga::{ReconcileVerdict, SagaOutcome};
use crate::workflow::NodeId;

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
    Branches,
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
    timed_out: bool,
}

impl StepState {
    const fn pending() -> Self {
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

    /// The branch a condition chose, once evaluated.
    #[must_use]
    pub const fn condition(&self) -> Option<bool> {
        self.condition
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
            timed_out,
        }
    }

    const fn fenced_by(&self, attempt: u32, generation: u64) -> bool {
        self.attempt == attempt && self.generation == generation
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
    sagas: BTreeMap<NodeId, SagaState>,
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
            sagas: BTreeMap::new(),
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
        Self {
            run_id,
            status,
            stop,
            paused,
            steps: steps
                .into_iter()
                .map(|(node, state)| (ExecutionInstanceId::root(node), state))
                .collect(),
            sagas,
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

    /// The sagas that have started, ordered by node id.
    pub fn sagas(&self) -> impl Iterator<Item = (&NodeId, &SagaState)> {
        self.sagas.iter()
    }

    #[must_use]
    pub fn saga(&self, node: &NodeId) -> Option<&SagaState> {
        self.sagas.get(node)
    }

    /// The status the run reaches when it finishes after being stopped for `reason`: the sagas that
    /// ran *because* the run was stopping can make it worse than the reason alone says (a
    /// cancellation whose compensation failed is not a clean cancellation, and a run that ended
    /// because one saga was compensated is not `COMPENSATED` if a sibling saga could not be).
    #[must_use]
    pub fn final_status_for(&self, reason: StopReason) -> RunStatus {
        let own = match reason {
            StopReason::Compensated => Some(SagaOutcome::Compensated),
            StopReason::Partial => Some(SagaOutcome::Partial),
            StopReason::CompensationFailed => Some(SagaOutcome::CompensationFailed),
            StopReason::Cancelled | StopReason::TimedOut | StopReason::Failed => None,
        };
        let worst = self
            .sagas
            .values()
            .filter(|saga| saga.during_stop)
            .filter_map(|saga| saga.outcome);
        match own.into_iter().chain(worst).max() {
            Some(SagaOutcome::CompensationFailed) => RunStatus::CompensationFailed,
            Some(SagaOutcome::Partial) => RunStatus::Partial,
            Some(SagaOutcome::Compensated) if own.is_some() => RunStatus::Compensated,
            _ => reason.final_status(),
        }
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
            RunEvent::ReconciliationRequested { node, fence, cause } => {
                let reconciliation = node.reconciliation();
                let reusable = match self
                    .steps
                    .get(&ExecutionInstanceId::root(reconciliation.clone()))
                {
                    None => true,
                    Some(step) => matches!(step.status, Succeeded | Failed),
                };
                if !reusable {
                    return Err(ExecutionError::AlreadyScheduled(reconciliation));
                }
                let step = self.fenced_step(
                    node,
                    name,
                    &[Leased, Running],
                    fence.attempt,
                    fence.generation,
                )?;
                step.status = Waiting;
                step.waiting = Some(WaitingOn::Reconciliation);
                step.failure = Some(cause.clone());
                step.cancel_requested = false;
                // A step that is reconciled again (after an attempt that was retried, or after an
                // operator asked for another one) reopens its reconciliation: the counters go on, so
                // the retry budget of the reconciliation is shared by every round.
                let companion = self
                    .steps
                    .entry(ExecutionInstanceId::root(reconciliation))
                    .or_insert_with(StepState::pending);
                companion.status = Pending;
                companion.routed = false;
                companion.waiting = None;
                companion.failure = None;
                companion.cancel_requested = false;
                companion.next_attempt_at = None;
            }
            RunEvent::ReconciliationConcluded {
                node,
                verdict,
                result,
            } => {
                let step = self.step_mut(node, name, &[Waiting])?;
                if !matches!(step.waiting, Some(WaitingOn::Reconciliation)) {
                    return Err(ExecutionError::IllegalTransition {
                        node: node.clone(),
                        status: step.status,
                        event: name,
                    });
                }
                let paired = matches!(
                    (verdict, result),
                    (ReconcileVerdict::Applied, ReconcileResult::Succeeded)
                        | (
                            ReconcileVerdict::NotApplied,
                            ReconcileResult::RetryAt(_) | ReconcileResult::Failed(_)
                        )
                        | (ReconcileVerdict::Unknown, ReconcileResult::Review(_))
                );
                if !paired {
                    return Err(ExecutionError::Invariant(
                        "a reconciliation result does not follow from what it found",
                    ));
                }
                step.waiting = None;
                match result {
                    ReconcileResult::Succeeded => {
                        step.status = Succeeded;
                        step.failure = None;
                    }
                    ReconcileResult::RetryAt(at) => {
                        step.status = RetryWait;
                        step.next_attempt_at = Some(*at);
                        // The lookup said it did not happen: the step is no longer one whose outcome
                        // is unknown, so a saga that stops while it backs off has nothing to undo.
                        step.failure = Some(FailureCode::reconciled_not_applied());
                    }
                    ReconcileResult::Failed(code) => {
                        step.status = Failed;
                        step.failure = Some(code.clone());
                    }
                    ReconcileResult::Review(code) => {
                        step.status = Waiting;
                        step.waiting = Some(WaitingOn::Operator { code: code.clone() });
                        step.failure = Some(code.clone());
                    }
                }
            }
            RunEvent::StepNeedsReview { node, code, fence } => {
                let step =
                    self.step_mut(node, name, &[Leased, Running, Waiting, Ready, RetryWait])?;
                match (step.status.is_leased(), fence, &step.waiting) {
                    (true, Some(f), _) if step.fenced_by(f.attempt, f.generation) => {}
                    (true, _, _) => {
                        return Err(ExecutionError::StaleAttempt { node: node.clone() });
                    }
                    (false, None, Some(WaitingOn::Reconciliation)) => {}
                    // An operator holds a step that has not started: it waits for a decision like
                    // the ones that ran out of attempts.
                    (false, None, None) if matches!(step.status, Ready | RetryWait) => {}
                    _ => {
                        return Err(ExecutionError::IllegalTransition {
                            node: node.clone(),
                            status: step.status,
                            event: name,
                        });
                    }
                }
                // An operator's hold of a step that has not started says why it waits now (the
                // code of the waiting); it does not say what happened to its last attempt. If that
                // attempt ended without anyone knowing whether the effect happened, the step still
                // says so: a saga that is stopped later must undo it.
                let held = fence.is_none() && matches!(step.status, Ready | RetryWait);
                step.status = Waiting;
                step.waiting = Some(WaitingOn::Operator { code: code.clone() });
                if !(held && step.failure.is_some()) {
                    step.failure = Some(code.clone());
                }
                step.cancel_requested = false;
                step.next_attempt_at = None;
            }
            RunEvent::StepRetryRequested { node } => {
                // A step that waits for a decision, or one whose retry is scheduled and an operator
                // does not want to wait for: the scheduler's own `retry_due` finds it no longer in
                // `RETRY_WAIT` and does nothing, so the two cannot both start an attempt.
                let step = self.step_mut(node, name, &[Waiting, RetryWait])?;
                if step.status == Waiting
                    && !matches!(step.waiting, Some(WaitingOn::Operator { .. }))
                {
                    return Err(ExecutionError::IllegalTransition {
                        node: node.clone(),
                        status: step.status,
                        event: name,
                    });
                }
                step.status = Ready;
                step.waiting = None;
                step.next_attempt_at = None;
            }
            RunEvent::StepSkipped { node } => {
                if !node.is_compensation() {
                    return Err(ExecutionError::Invariant(
                        "only a compensation can be skipped",
                    ));
                }
                let step = self.step_mut(node, name, &[Waiting])?;
                if !matches!(step.waiting, Some(WaitingOn::Operator { .. })) {
                    return Err(ExecutionError::IllegalTransition {
                        node: node.clone(),
                        status: step.status,
                        event: name,
                    });
                }
                step.status = Skipped;
                step.waiting = None;
            }
            RunEvent::StepResolved { node, outcome } => {
                let step = self.step_mut(node, name, &[Waiting])?;
                if !matches!(step.waiting, Some(WaitingOn::Operator { .. })) {
                    return Err(ExecutionError::IllegalTransition {
                        node: node.clone(),
                        status: step.status,
                        event: name,
                    });
                }
                step.waiting = None;
                match outcome {
                    ReviewOutcome::Succeeded => {
                        step.status = Succeeded;
                        step.failure = None;
                    }
                    ReviewOutcome::Failed(code) => {
                        step.status = Failed;
                        step.failure = Some(code.clone());
                    }
                }
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
            RunEvent::RunFinished { status } => self.check_finish(*status)?,
        }
        self.applied += 1;
        self.refresh_status(event);
        Ok(())
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
        if let RunEvent::RunFinished { status } = event {
            self.status = *status;
            return;
        }
        let active = |s: &StepState| {
            matches!(
                s.status,
                StepStatus::Pending
                    | StepStatus::Ready
                    | StepStatus::Leased
                    | StepStatus::Running
                    | StepStatus::RetryWait
            )
        };
        // Something can still move by itself.
        let live = |s: &StepState| {
            matches!(
                s.status,
                StepStatus::Ready
                    | StepStatus::Leased
                    | StepStatus::Running
                    | StepStatus::RetryWait
            )
        };
        let reviewing = self.steps.values().any(|s| {
            s.status == StepStatus::Waiting && matches!(s.waiting, Some(WaitingOn::Operator { .. }))
        });
        let compensating = self
            .sagas
            .values()
            .any(|s| matches!(s.phase, SagaPhase::Aborting | SagaPhase::Compensating));
        self.status = if reviewing && !self.steps.values().any(live) {
            RunStatus::ManualReview
        } else if compensating {
            RunStatus::Compensating
        } else if self.stop.is_some() {
            RunStatus::Cancelling
        } else if self.paused {
            RunStatus::Paused
        } else if !self.steps.values().any(active)
            && self.steps.values().any(|s| s.status == StepStatus::Waiting)
        {
            RunStatus::Waiting
        } else {
            RunStatus::Running
        };
    }
}
