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
use super::event::{CorrelationKey, FailureCode, JoinOutcome, RunEvent, StopReason};
use crate::identity::IdentityError;
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
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

impl RunStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "RUNNING",
            Self::Waiting => "WAITING",
            Self::Paused => "PAUSED",
            Self::Cancelling => "CANCELLING",
            Self::Succeeded => "SUCCEEDED",
            Self::Failed => "FAILED",
            Self::Cancelled => "CANCELLED",
            Self::TimedOut => "TIMED_OUT",
        }
    }

    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::TimedOut
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
            "SUCCEEDED" => Ok(Self::Succeeded),
            "FAILED" => Ok(Self::Failed),
            "CANCELLED" => Ok(Self::Cancelled),
            "TIMED_OUT" => Ok(Self::TimedOut),
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
        }
    }

    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
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
    steps: BTreeMap<NodeId, StepState>,
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
    pub const fn restore(
        run_id: RunId,
        status: RunStatus,
        stop: Option<(StopReason, Option<FailureCode>)>,
        paused: bool,
        steps: BTreeMap<NodeId, StepState>,
        version: u64,
    ) -> Self {
        Self {
            run_id,
            status,
            stop,
            paused,
            steps,
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
        self.steps.get(node)
    }

    /// All steps, ordered by node id.
    pub fn steps(&self) -> impl Iterator<Item = (&NodeId, &StepState)> {
        self.steps.iter()
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
            .get_mut(node)
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
            if self.steps.contains_key(node) || nodes[..i].contains(node) {
                return Err(ExecutionError::AlreadyScheduled(node.clone()));
            }
        }
        for node in nodes {
            self.steps.insert(node.clone(), StepState::pending());
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
            Cancelled, Failed, Leased, Pending, Ready, RetryWait, Running, Succeeded, Waiting,
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
                let step = self.step_mut(node, name, &[Succeeded, Failed])?;
                if step.routed {
                    return Err(ExecutionError::IllegalTransition {
                        node: node.clone(),
                        status: step.status,
                        event: name,
                    });
                }
                self.activate(activated)?;
                if let Some(step) = self.steps.get_mut(node) {
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
                if let Some(step) = self.steps.get_mut(node) {
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
            RunEvent::EndReached { node } => {
                let step = self.step_mut(node, name, &[Pending])?;
                step.status = Succeeded;
                step.routed = true;
            }
            RunEvent::StepCancelRequested { node } => {
                let step = self.step_mut(node, name, &[Leased, Running])?;
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
            Some((reason, _)) if reason.final_status() == status => Ok(()),
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
        self.status = if self.stop.is_some() {
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
