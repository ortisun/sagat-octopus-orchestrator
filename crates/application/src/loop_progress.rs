//! Transaction boundary for durable sequential-loop checkpoints and delays.
//!
//! The port records only bounded scheduler decisions. Expression evaluation happens before the
//! transaction, while the adapter rechecks the run revision, scheduler fence and persisted scope.
//! Workflow publication stays gated until every scheduler failure path has durable evidence.

use std::future::Future;

use sagat_domain::audit::AuditRecord;
use sagat_domain::execution::{
    ExecutionInstanceId, LoopContinuation, LoopDrainReason, LoopKind, RunId,
};
use sagat_domain::identity::{NamespaceId, Subject, TenantId};
use sagat_domain::workflow::{ActivityName, Digest};
use sha2::{Digest as _, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::StoreError;
use crate::maintenance::{StepRef, Window};
use crate::primitives::SchedulerAuthority;

/// Values selected once when a loop owner is first evaluated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopEntryEvaluation {
    pub carry: Vec<u8>,
    pub condition: Option<bool>,
}

/// Values selected together after a loop body reaches its return boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopCheckpointEvaluation {
    pub result: Vec<u8>,
    pub carry: Vec<u8>,
    pub condition: bool,
}

/// One loop owner found by the scheduler's read-only discovery query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopCandidate {
    pub step: StepRef,
    pub timer_id: Option<Uuid>,
}

/// Scheduler-owned context shared by one loop progress transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopTransition {
    tenant_id: TenantId,
    namespace_id: NamespaceId,
    run_id: RunId,
    expected_run_version: u64,
    owner: ExecutionInstanceId,
    scheduler: SchedulerAuthority,
    actor: Subject,
    trace_id: Option<String>,
    occurred_at: OffsetDateTime,
    audit: AuditRecord,
}

impl LoopTransition {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        tenant_id: TenantId,
        namespace_id: NamespaceId,
        run_id: RunId,
        expected_run_version: u64,
        owner: ExecutionInstanceId,
        scheduler: SchedulerAuthority,
        actor: Subject,
        trace_id: Option<String>,
        occurred_at: OffsetDateTime,
        audit: AuditRecord,
    ) -> Self {
        Self {
            tenant_id,
            namespace_id,
            run_id,
            expected_run_version,
            owner,
            scheduler,
            actor,
            trace_id,
            occurred_at,
            audit,
        }
    }

    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    #[must_use]
    pub const fn namespace_id(&self) -> NamespaceId {
        self.namespace_id
    }

    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.run_id
    }

    #[must_use]
    pub const fn expected_run_version(&self) -> u64 {
        self.expected_run_version
    }

    #[must_use]
    pub const fn owner(&self) -> &ExecutionInstanceId {
        &self.owner
    }

    #[must_use]
    pub const fn scheduler(&self) -> &SchedulerAuthority {
        &self.scheduler
    }

    #[must_use]
    pub const fn actor(&self) -> &Subject {
        &self.actor
    }

    #[must_use]
    pub fn trace_id(&self) -> Option<&str> {
        self.trace_id.as_deref()
    }

    #[must_use]
    pub const fn occurred_at(&self) -> OffsetDateTime {
        self.occurred_at
    }

    #[must_use]
    pub const fn audit(&self) -> &AuditRecord {
        &self.audit
    }
}

/// The next activity instance admitted by a checkpoint or an elapsed delay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopNextActivity {
    pub instance: ExecutionInstanceId,
    pub activity: ActivityName,
    pub reserved_instances: u16,
}

impl LoopNextActivity {
    /// # Errors
    /// Returns [`LoopInputError::MissingReturnReservation`] when the body cannot retain its return.
    pub fn new(
        instance: ExecutionInstanceId,
        activity: ActivityName,
        reserved_instances: u16,
    ) -> Result<Self, LoopInputError> {
        if reserved_instances == 0 {
            return Err(LoopInputError::MissingReturnReservation);
        }
        Ok(Self {
            instance,
            activity,
            reserved_instances,
        })
    }
}

/// The first durable decision for one sequential loop activation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopEntryCommand {
    pub transition: LoopTransition,
    pub kind: LoopKind,
    pub max_iterations: u32,
    pub deadline: OffsetDateTime,
    pub initial_carry_digest: Digest,
    pub initial_condition: Option<bool>,
    pub first_activity: Option<LoopNextActivity>,
    pub outbox_id: Uuid,
    initial_carry_content: Option<Vec<u8>>,
}

impl LoopEntryCommand {
    /// # Errors
    /// Returns [`LoopInputError::Entry`] when the loop bounds, predicate phase or first iteration
    /// do not form one complete entry decision.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        transition: LoopTransition,
        kind: LoopKind,
        max_iterations: u32,
        deadline: OffsetDateTime,
        initial_carry_digest: Digest,
        initial_condition: Option<bool>,
        first_activity: Option<LoopNextActivity>,
        outbox_id: Uuid,
    ) -> Result<Self, LoopInputError> {
        let timeout_ms = (deadline - transition.occurred_at()).whole_milliseconds();
        let valid_phase = matches!(
            (kind, initial_condition),
            (LoopKind::While, Some(_)) | (LoopKind::RepeatUntil, None)
        );
        let needs_body = kind == LoopKind::RepeatUntil || initial_condition == Some(true);
        let expected_path = transition
            .owner()
            .path()
            .child(transition.owner().node().clone(), 0)
            .map_err(|_| LoopInputError::InstancePath)?;
        let valid_activity = first_activity
            .as_ref()
            .is_some_and(|first| first.instance.path() == &expected_path);
        if !(1..=10_000).contains(&max_iterations)
            || !(1..=31_622_400_000).contains(&timeout_ms)
            || !valid_phase
            || needs_body != valid_activity
        {
            return Err(LoopInputError::Entry);
        }
        Ok(Self {
            transition,
            kind,
            max_iterations,
            deadline,
            initial_carry_digest,
            initial_condition,
            first_activity,
            outbox_id,
            initial_carry_content: None,
        })
    }

    /// Constructs a runtime entry whose initial carry must commit with the loop transition.
    ///
    /// # Errors
    /// Returns [`LoopInputError::Entry`] when the loop entry decision is invalid.
    #[allow(clippy::too_many_arguments)]
    pub fn from_carry(
        transition: LoopTransition,
        kind: LoopKind,
        max_iterations: u32,
        deadline: OffsetDateTime,
        initial_carry_content: Vec<u8>,
        initial_condition: Option<bool>,
        first_activity: Option<LoopNextActivity>,
        outbox_id: Uuid,
    ) -> Result<Self, LoopInputError> {
        let initial_carry_digest = payload_digest(&initial_carry_content);
        let mut command = Self::new(
            transition,
            kind,
            max_iterations,
            deadline,
            initial_carry_digest,
            initial_condition,
            first_activity,
            outbox_id,
        )?;
        command.initial_carry_content = Some(initial_carry_content);
        Ok(command)
    }

    #[must_use]
    pub fn initial_carry_content(&self) -> Option<&[u8]> {
        self.initial_carry_content.as_deref()
    }
}

/// One body return, its evaluated checkpoint and its already-decided continuation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopCheckpointCommand {
    pub transition: LoopTransition,
    pub index: u32,
    pub return_instance: ExecutionInstanceId,
    pub result_digest: Digest,
    pub carry_digest: Digest,
    pub condition: bool,
    pub checked_at: OffsetDateTime,
    pub continuation: LoopContinuation,
    pub next_activity: Option<LoopNextActivity>,
    pub timer_id: Option<Uuid>,
    pub outbox_id: Uuid,
    result_content: Option<Vec<u8>>,
    carry_content: Option<Vec<u8>>,
}

impl LoopCheckpointCommand {
    /// # Errors
    /// Returns [`LoopInputError`] when the continuation lacks its required durable effect or an
    /// instance does not belong to the expected loop iteration.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        transition: LoopTransition,
        index: u32,
        return_instance: ExecutionInstanceId,
        result_digest: Digest,
        carry_digest: Digest,
        condition: bool,
        checked_at: OffsetDateTime,
        continuation: LoopContinuation,
        next_activity: Option<LoopNextActivity>,
        timer_id: Option<Uuid>,
        outbox_id: Uuid,
    ) -> Result<Self, LoopInputError> {
        let member_path = transition
            .owner()
            .path()
            .child(transition.owner().node().clone(), index)
            .map_err(|_| LoopInputError::InstancePath)?;
        if return_instance.path() != &member_path {
            return Err(LoopInputError::InstancePath);
        }
        let next_index = index.checked_add(1).ok_or(LoopInputError::Continuation)?;
        let valid_effect = match continuation {
            LoopContinuation::Continue => {
                timer_id.is_none()
                    && next_activity.as_ref().is_some_and(|next| {
                        transition
                            .owner()
                            .path()
                            .child(transition.owner().node().clone(), next_index)
                            .is_ok_and(|path| next.instance.path() == &path)
                    })
            }
            LoopContinuation::DelayUntil(due_at) => {
                due_at > checked_at && timer_id.is_some() && next_activity.is_none()
            }
            LoopContinuation::Complete
            | LoopContinuation::IterationLimit
            | LoopContinuation::InstanceLimit => timer_id.is_none() && next_activity.is_none(),
        };
        if !valid_effect {
            return Err(LoopInputError::Continuation);
        }
        Ok(Self {
            transition,
            index,
            return_instance,
            result_digest,
            carry_digest,
            condition,
            checked_at,
            continuation,
            next_activity,
            timer_id,
            outbox_id,
            result_content: None,
            carry_content: None,
        })
    }

    /// Constructs a runtime checkpoint whose result and carry must commit with the transition.
    ///
    /// # Errors
    /// Returns [`LoopInputError`] when the continuation or an instance path is invalid.
    #[allow(clippy::too_many_arguments)]
    pub fn from_evaluation(
        transition: LoopTransition,
        index: u32,
        return_instance: ExecutionInstanceId,
        evaluation: LoopCheckpointEvaluation,
        checked_at: OffsetDateTime,
        continuation: LoopContinuation,
        next_activity: Option<LoopNextActivity>,
        timer_id: Option<Uuid>,
        outbox_id: Uuid,
    ) -> Result<Self, LoopInputError> {
        let result_digest = payload_digest(&evaluation.result);
        let carry_digest = payload_digest(&evaluation.carry);
        let mut command = Self::new(
            transition,
            index,
            return_instance,
            result_digest,
            carry_digest,
            evaluation.condition,
            checked_at,
            continuation,
            next_activity,
            timer_id,
            outbox_id,
        )?;
        command.result_content = Some(evaluation.result);
        command.carry_content = Some(evaluation.carry);
        Ok(command)
    }

    #[must_use]
    pub fn result_content(&self) -> Option<&[u8]> {
        self.result_content.as_deref()
    }

    #[must_use]
    pub fn carry_content(&self) -> Option<&[u8]> {
        self.carry_content.as_deref()
    }
}

fn payload_digest(content: &[u8]) -> Digest {
    Digest::new(Sha256::digest(content).into())
}

/// An elapsed durable delay and the next body activity admitted in the same transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopDelayElapsedCommand {
    pub transition: LoopTransition,
    pub timer_id: Uuid,
    pub next_iteration: u32,
    pub next_activity: LoopNextActivity,
    pub outbox_id: Uuid,
}

/// Starts a durable loop drain and stops further admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopDrainCommand {
    pub transition: LoopTransition,
    pub reason: LoopDrainReason,
    pub outbox_id: Uuid,
}

impl LoopDrainCommand {
    /// # Errors
    /// Returns [`LoopInputError::DrainIteration`] when a failed iteration is outside the loop bound.
    pub fn new(
        transition: LoopTransition,
        reason: LoopDrainReason,
        outbox_id: Uuid,
    ) -> Result<Self, LoopInputError> {
        if matches!(reason, LoopDrainReason::Failed { iteration, .. } if iteration > 9_999) {
            return Err(LoopInputError::DrainIteration);
        }
        Ok(Self {
            transition,
            reason,
            outbox_id,
        })
    }
}

/// Releases one settled active iteration while its loop is draining.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopIterationDrainCommand {
    pub transition: LoopTransition,
    pub index: u32,
    pub outbox_id: Uuid,
}

impl LoopIterationDrainCommand {
    /// # Errors
    /// Returns [`LoopInputError::DrainIteration`] when the index is outside the loop bound.
    pub fn new(
        transition: LoopTransition,
        index: u32,
        outbox_id: Uuid,
    ) -> Result<Self, LoopInputError> {
        if index > 9_999 {
            return Err(LoopInputError::DrainIteration);
        }
        Ok(Self {
            transition,
            index,
            outbox_id,
        })
    }
}

/// Closes a drained loop with the outcome recorded when drain began.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopDrainFinishCommand {
    pub transition: LoopTransition,
    pub outbox_id: Uuid,
}

impl LoopDrainFinishCommand {
    #[must_use]
    pub const fn new(transition: LoopTransition, outbox_id: Uuid) -> Self {
        Self {
            transition,
            outbox_id,
        }
    }
}

impl LoopDelayElapsedCommand {
    /// # Errors
    /// Returns [`LoopInputError::InstancePath`] if the next activity is outside the next iteration.
    pub fn new(
        transition: LoopTransition,
        timer_id: Uuid,
        next_iteration: u32,
        next_activity: LoopNextActivity,
        outbox_id: Uuid,
    ) -> Result<Self, LoopInputError> {
        let expected = transition
            .owner()
            .path()
            .child(transition.owner().node().clone(), next_iteration)
            .map_err(|_| LoopInputError::InstancePath)?;
        if next_activity.instance.path() != &expected {
            return Err(LoopInputError::InstancePath);
        }
        Ok(Self {
            transition,
            timer_id,
            next_iteration,
            next_activity,
            outbox_id,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopWrite {
    pub applied: bool,
    pub run_version: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LoopInputError {
    #[error("the initial loop decision is incomplete or outside its bounds")]
    Entry,
    #[error("the instance does not belong to the expected loop iteration")]
    InstancePath,
    #[error("an admitted loop body must reserve its return instance")]
    MissingReturnReservation,
    #[error("the loop continuation does not match its durable effect")]
    Continuation,
    #[error("the loop drain iteration is outside the supported bound")]
    DrainIteration,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoopProgressError {
    #[error("run or loop progress was not found")]
    Missing,
    #[error("run version changed (current: {current})")]
    RevisionConflict { current: u64 },
    #[error("the loop transition is not legal in its current state")]
    InvalidTransition,
    #[error("the same loop receipt was reused for different data")]
    DuplicateMismatch,
    #[error("tenant iteration capacity is currently full")]
    AtCapacity,
    #[error("the run instance budget is exhausted")]
    InstanceBudgetExceeded,
    #[error("audit evidence does not describe this loop transition")]
    AuditTenantMismatch,
    #[error("tenant iteration policy is not configured")]
    PolicyMissing,
    #[error("scheduler ownership changed before the transition committed")]
    SchedulerFenceLost,
    #[error("loop completion routing is not integrated with durable replay")]
    RuntimeNotIntegrated,
    #[error(transparent)]
    Storage(#[from] StoreError),
}

/// Durable loop checkpoint and delay storage. Matching replays never charge or notify twice.
pub trait LoopProgressStore: Send + Sync {
    fn loop_candidates(
        &self,
        window: &Window,
    ) -> impl Future<Output = Result<Vec<LoopCandidate>, LoopProgressError>> + Send;

    fn enter_loop(
        &self,
        command: &LoopEntryCommand,
    ) -> impl Future<Output = Result<LoopWrite, LoopProgressError>> + Send;

    fn checkpoint(
        &self,
        command: &LoopCheckpointCommand,
    ) -> impl Future<Output = Result<LoopWrite, LoopProgressError>> + Send;

    fn fire_delay(
        &self,
        command: &LoopDelayElapsedCommand,
    ) -> impl Future<Output = Result<LoopWrite, LoopProgressError>> + Send;

    fn start_drain(
        &self,
        command: &LoopDrainCommand,
    ) -> impl Future<Output = Result<LoopWrite, LoopProgressError>> + Send;

    fn drain_iteration(
        &self,
        command: &LoopIterationDrainCommand,
    ) -> impl Future<Output = Result<LoopWrite, LoopProgressError>> + Send;

    fn finish_drain(
        &self,
        command: &LoopDrainFinishCommand,
    ) -> impl Future<Output = Result<LoopWrite, LoopProgressError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;
    use sagat_domain::audit::{AuditAction, ResourceType};
    use sagat_domain::execution::{InstancePath, RunId};
    use sagat_domain::identity::{Principal, Role};
    use sagat_domain::workflow::NodeId;

    fn transition(owner: ExecutionInstanceId) -> LoopTransition {
        let tenant = TenantId::generate();
        let run = RunId::generate();
        let principal = Principal::new(tenant, Subject::engine(), Role::Admin);
        LoopTransition::new(
            tenant,
            NamespaceId::generate(),
            run,
            1,
            owner,
            SchedulerAuthority::for_run("scheduler", run, 1),
            Subject::engine(),
            None,
            OffsetDateTime::UNIX_EPOCH,
            AuditRecord::by(
                &principal,
                AuditAction::RunUpdated,
                ResourceType::Run,
                run.to_string(),
                None,
                OffsetDateTime::UNIX_EPOCH,
            ),
        )
    }

    #[test]
    fn continuation_requires_exactly_one_matching_durable_effect() {
        let owner_node = NodeId::parse("poll").expect("owner");
        let owner = ExecutionInstanceId::root(owner_node.clone());
        let member = InstancePath::root()
            .child(owner_node.clone(), 0)
            .expect("member");
        let returned =
            ExecutionInstanceId::in_path(member, NodeId::parse("returned").expect("return"));
        let next = LoopNextActivity::new(
            ExecutionInstanceId::in_path(
                InstancePath::root()
                    .child(owner_node, 1)
                    .expect("next member"),
                NodeId::parse("work").expect("entry"),
            ),
            ActivityName::parse("poll.check").expect("activity"),
            1,
        )
        .expect("next activity");
        let digest = Digest::new([7; 32]);

        assert!(
            LoopCheckpointCommand::new(
                transition(owner.clone()),
                0,
                returned.clone(),
                digest,
                digest,
                true,
                OffsetDateTime::UNIX_EPOCH,
                LoopContinuation::Continue,
                Some(next),
                None,
                Uuid::now_v7(),
            )
            .is_ok()
        );
        assert_eq!(
            LoopCheckpointCommand::new(
                transition(owner),
                0,
                returned,
                digest,
                digest,
                true,
                OffsetDateTime::UNIX_EPOCH,
                LoopContinuation::DelayUntil(OffsetDateTime::UNIX_EPOCH),
                None,
                Some(Uuid::now_v7()),
                Uuid::now_v7(),
            ),
            Err(LoopInputError::Continuation)
        );

        let evaluation = LoopCheckpointEvaluation {
            result: br#"{"done":true}"#.to_vec(),
            carry: b"3".to_vec(),
            condition: false,
        };
        let command = LoopCheckpointCommand::from_evaluation(
            transition(ExecutionInstanceId::root(
                NodeId::parse("poll").expect("owner"),
            )),
            0,
            ExecutionInstanceId::in_path(
                InstancePath::root()
                    .child(NodeId::parse("poll").expect("owner"), 0)
                    .expect("member"),
                NodeId::parse("returned").expect("return"),
            ),
            evaluation.clone(),
            OffsetDateTime::UNIX_EPOCH,
            LoopContinuation::Complete,
            None,
            None,
            Uuid::now_v7(),
        )
        .expect("runtime checkpoint");
        assert_eq!(command.result_content(), Some(evaluation.result.as_slice()));
        assert_eq!(command.carry_content(), Some(evaluation.carry.as_slice()));
        assert_eq!(command.result_digest, payload_digest(&evaluation.result));
        assert_eq!(command.carry_digest, payload_digest(&evaluation.carry));
    }

    #[test]
    fn entry_requires_the_declared_predicate_phase_and_first_iteration() {
        let owner_node = NodeId::parse("poll").expect("owner");
        let owner = ExecutionInstanceId::root(owner_node.clone());
        let first = LoopNextActivity::new(
            ExecutionInstanceId::in_path(
                InstancePath::root()
                    .child(owner_node, 0)
                    .expect("first member"),
                NodeId::parse("work").expect("entry"),
            ),
            ActivityName::parse("poll.check").expect("activity"),
            1,
        )
        .expect("first activity");
        let base_transition = transition(owner);
        let deadline = base_transition.occurred_at() + time::Duration::seconds(1);
        let digest = Digest::new([7; 32]);

        assert!(
            LoopEntryCommand::new(
                base_transition.clone(),
                LoopKind::While,
                3,
                deadline,
                digest,
                Some(true),
                Some(first.clone()),
                Uuid::now_v7(),
            )
            .is_ok()
        );
        assert!(
            LoopEntryCommand::new(
                base_transition.clone(),
                LoopKind::While,
                3,
                deadline,
                digest,
                Some(false),
                None,
                Uuid::now_v7(),
            )
            .is_ok()
        );
        assert_eq!(
            LoopEntryCommand::new(
                base_transition.clone(),
                LoopKind::RepeatUntil,
                3,
                deadline,
                digest,
                Some(true),
                Some(first.clone()),
                Uuid::now_v7(),
            ),
            Err(LoopInputError::Entry)
        );
        assert_eq!(
            LoopEntryCommand::new(
                base_transition,
                LoopKind::RepeatUntil,
                3,
                deadline,
                digest,
                None,
                None,
                Uuid::now_v7(),
            ),
            Err(LoopInputError::Entry)
        );

        let carry = b"0".to_vec();
        let command = LoopEntryCommand::from_carry(
            transition(ExecutionInstanceId::root(
                NodeId::parse("repeat").expect("owner"),
            )),
            LoopKind::RepeatUntil,
            3,
            OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(1),
            carry.clone(),
            None,
            Some(
                LoopNextActivity::new(
                    ExecutionInstanceId::in_path(
                        InstancePath::root()
                            .child(NodeId::parse("repeat").expect("owner"), 0)
                            .expect("member"),
                        NodeId::parse("work").expect("entry"),
                    ),
                    ActivityName::parse("poll.check").expect("activity"),
                    1,
                )
                .expect("first activity"),
            ),
            Uuid::now_v7(),
        )
        .expect("runtime entry");
        assert_eq!(command.initial_carry_content(), Some(carry.as_slice()));
        assert_eq!(command.initial_carry_digest, payload_digest(&carry));
    }
}
