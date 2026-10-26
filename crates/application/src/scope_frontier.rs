//! The bounded read model a collection or loop transition decides from (G461, D520).
//!
//! A scheduler transition needs the counters of one scope, the iterations that are in flight (at
//! most `maxConcurrency` for a collection, one for a loop) and the steps of those iterations. It
//! does not need the iterations that already ended: they are settled, immutable evidence. Reading
//! the whole run to find that out made the cost of a transition grow with the progress of the scope.
//!
//! A frontier is read in one snapshot and carries the run version it was read at. Every write the
//! scheduler then makes locks the run, checks its fence and compares that version, so a frontier
//! that went stale is refused by the write, never acted on.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use sagat_domain::execution::{
    CollectionPhase, ExecutionInstanceId, FailureCode, LoopPhase, RunId, StepState, StepStatus,
    StopReason,
};
use sagat_domain::identity::TenantId;
use sagat_domain::workflow::{NodeKind, WorkflowDefinition};
use time::OffsetDateTime;

use crate::error::StoreError;
use crate::execution::{RunData, RunInfo};
use crate::primitives::CollectionScopeId;

/// What the run itself contributes to a scope transition.
#[derive(Debug, Clone)]
pub struct ScopeRun {
    pub info: RunInfo,
    /// The database clock when the snapshot was taken (the instant the transition is stamped with).
    pub now: OffsetDateTime,
    /// The journal head the snapshot was read at; the writes compare it under the run's row lock.
    pub version: u64,
    pub stop_reason: Option<StopReason>,
    /// The typed failure paired with the stop, when the stored Run has one.
    pub stop_code: Option<FailureCode>,
    /// The stored logical-output ledger plus verified legacy root-collection owner outputs.
    pub effective_output_bytes: u64,
    pub definition: Arc<WorkflowDefinition>,
    /// Instances charged against the run's lifetime budget: the same number as
    /// `RunState::charged_instances` (D522).
    pub charged_instances: usize,
}

/// One iteration in flight and every step that exists in its scope path.
#[derive(Debug, Clone)]
pub struct ScopeMember {
    pub index: u32,
    pub entry: ExecutionInstanceId,
    pub steps: BTreeMap<ExecutionInstanceId, StepState>,
}

impl ScopeMember {
    /// The code of the first failed step of the member, in instance order.
    #[must_use]
    pub fn first_failure(&self) -> Option<&FailureCode> {
        self.steps
            .values()
            .find(|step| step.status() == StepStatus::Failed && step.failure().is_some())
            .and_then(StepState::failure)
    }

    /// The member's `return` step that is waiting for the scheduler to checkpoint it.
    #[must_use]
    pub fn pending_return(&self, definition: &WorkflowDefinition) -> Option<&ExecutionInstanceId> {
        self.steps.iter().find_map(|(instance, step)| {
            (step.status() == StepStatus::Pending
                && matches!(
                    definition
                        .node_for_instance(instance)
                        .map(sagat_domain::workflow::Node::kind),
                    Some(NodeKind::Return { .. })
                ))
            .then_some(instance)
        })
    }
}

/// The part of a collection scope a transition decides from.
#[derive(Debug, Clone)]
pub struct CollectionFrontier {
    pub run: ScopeRun,
    pub scope_id: CollectionScopeId,
    pub phase: CollectionPhase,
    pub item_count: u32,
    pub next_admission: u32,
    /// Iterations that ended with a result. In an open scope every ended iteration is one of them.
    pub completed: u32,
    pub max_concurrency: u16,
    /// The iterations in flight, in index order.
    pub active: Vec<ScopeMember>,
    /// Whether root work other than the scope's owner is still open (only read while draining).
    pub other_root_work: bool,
    /// The run input, the declared state, and the outputs and inputs of the active members only.
    pub data: RunData,
}

/// The part of a loop scope a transition decides from.
#[derive(Debug, Clone)]
pub struct LoopFrontier {
    pub run: ScopeRun,
    pub phase: LoopPhase,
    pub deadline: OffsetDateTime,
    pub next_iteration: u32,
    pub delay_until: Option<OffsetDateTime>,
    /// The iteration in flight, if any (at most one).
    pub member: Option<ScopeMember>,
    /// Whether root work other than the loop's owner is still open (only read while draining).
    pub other_root_work: bool,
    /// The run input, the declared state, and the output and input of the active member only.
    pub data: RunData,
}

/// Reads the frontier of a scope without restoring the run.
///
/// `None` means the run or the scope does not exist yet, which is when the scheduler must use the
/// whole-run path (it enters scopes there).
pub trait ScopeFrontierStore: Send + Sync {
    fn collection_frontier(
        &self,
        tenant_id: TenantId,
        run_id: RunId,
        owner: &ExecutionInstanceId,
    ) -> impl Future<Output = Result<Option<CollectionFrontier>, StoreError>> + Send;

    fn loop_frontier(
        &self,
        tenant_id: TenantId,
        run_id: RunId,
        owner: &ExecutionInstanceId,
    ) -> impl Future<Output = Result<Option<LoopFrontier>, StoreError>> + Send;

    /// Effective bytes retained by the Run: actual step-output references plus verified successful
    /// root-collection aggregates whose owner output is absent. Read when a loop ends, to prove the
    /// carry it leaves fits beside them.
    fn retained_output_bytes(
        &self,
        tenant_id: TenantId,
        run_id: RunId,
    ) -> impl Future<Output = Result<u64, StoreError>> + Send;
}
