//! Bounded state for one admitted execution path.
//!
//! The complete [`RunState`](super::RunState) remains the replay and verification oracle. Runtime
//! progress can use this projection because it owns every step and nested scope in one exact path,
//! plus typed summaries of each ancestor and the global run fences that can stop local progress.

use std::collections::BTreeMap;

use super::{
    AddressedRunEvent, CollectionDrainReason, CollectionPhase, CollectionState, ExecutionError,
    ExecutionInstanceId, FailureCode, InstancePath, LoopDrainReason, LoopKind, LoopPhase,
    LoopState, RunEvent, RunId, RunState, RunStatus, StepState, StepStatus, StopReason,
};
use crate::saga::SagaOutcome;
use crate::workflow::{Digest, NodeId};

/// Scalar progress for one collection owned by the exact local path.
///
/// Historical members stay in their indexed projection; this state is independent of the number
/// of admitted iterations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathCollectionState {
    phase: CollectionPhase,
    item_count: u32,
    max_items: u32,
    max_concurrency: u16,
    snapshot_digest: Digest,
    next_admission: u32,
    completed_count: u32,
    active_count: u16,
    result_digest: Option<Digest>,
    drain_reason: Option<CollectionDrainReason>,
}

impl PathCollectionState {
    /// Restores trusted scalar collection progress.
    ///
    /// # Errors
    /// The counters exceed the collection bounds or contradict one another.
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        phase: CollectionPhase,
        item_count: u32,
        max_items: u32,
        max_concurrency: u16,
        snapshot_digest: Digest,
        next_admission: u32,
        completed_count: u32,
        active_count: u16,
        result_digest: Option<Digest>,
        drain_reason: Option<CollectionDrainReason>,
    ) -> Result<Self, ExecutionError> {
        if max_items == 0
            || max_concurrency == 0
            || item_count > max_items
            || next_admission > item_count
            || completed_count > next_admission
            || u32::from(active_count) > u32::from(max_concurrency)
            || u32::from(active_count) > next_admission - completed_count
        {
            return Err(ExecutionError::Invariant(
                "bounded collection has invalid scalar progress",
            ));
        }
        Ok(Self {
            phase,
            item_count,
            max_items,
            max_concurrency,
            snapshot_digest,
            next_admission,
            completed_count,
            active_count,
            result_digest,
            drain_reason,
        })
    }

    fn from_collection(state: &CollectionState) -> Result<Self, ExecutionError> {
        Self::restore(
            state.phase(),
            state.item_count(),
            state.max_items(),
            state.max_concurrency(),
            state.snapshot_digest(),
            state.next_admission(),
            u32::try_from(state.completed_count()).map_err(|_| {
                ExecutionError::Invariant("bounded collection completed count exceeded its bounds")
            })?,
            u16::try_from(state.active_count()).map_err(|_| {
                ExecutionError::Invariant("bounded collection active count exceeded its bounds")
            })?,
            state.result_digest(),
            state.drain_reason().cloned(),
        )
    }

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
    pub const fn admitted_count(&self) -> u32 {
        self.next_admission
    }

    #[must_use]
    pub const fn completed_count(&self) -> u32 {
        self.completed_count
    }

    #[must_use]
    pub const fn active_count(&self) -> u16 {
        self.active_count
    }

    #[must_use]
    pub const fn result_digest(&self) -> Option<Digest> {
        self.result_digest
    }

    #[must_use]
    pub const fn drain_reason(&self) -> Option<&CollectionDrainReason> {
        self.drain_reason.as_ref()
    }
}

/// Scalar progress for one loop owned by the exact local path.
///
/// Historical checkpoints stay outside this state; only the current iteration or delay can affect
/// forward progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathLoopState {
    kind: LoopKind,
    phase: LoopPhase,
    max_iterations: u32,
    entered_at: time::OffsetDateTime,
    deadline: time::OffsetDateTime,
    initial_carry_digest: Digest,
    current_carry_digest: Digest,
    next_iteration: u32,
    active_iteration: Option<u32>,
    delay_until: Option<time::OffsetDateTime>,
    drain_reason: Option<LoopDrainReason>,
    failure: Option<FailureCode>,
}

/// Bounded compensation authority relevant to one exact execution path.
///
/// The saga header belongs to `owner`. `entries` and `irreversible` contain only forward
/// instances in the path represented by [`PathExecutionState`]. Entries from sibling iterations
/// remain in their indexed projection and are deliberately absent here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathScopeSagaState {
    owner: ExecutionInstanceId,
    phase: super::SagaPhase,
    cause: FailureCode,
    entries: BTreeMap<NodeId, Vec<ExecutionInstanceId>>,
    irreversible: Vec<NodeId>,
    outcome: Option<SagaOutcome>,
    during_stop: bool,
}

impl PathScopeSagaState {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        owner: ExecutionInstanceId,
        phase: super::SagaPhase,
        cause: FailureCode,
        entries: BTreeMap<NodeId, Vec<ExecutionInstanceId>>,
        irreversible: Vec<NodeId>,
        outcome: Option<SagaOutcome>,
        during_stop: bool,
    ) -> Self {
        Self {
            owner,
            phase,
            cause,
            entries,
            irreversible,
            outcome,
            during_stop,
        }
    }

    #[must_use]
    pub const fn owner(&self) -> &ExecutionInstanceId {
        &self.owner
    }

    #[must_use]
    pub const fn phase(&self) -> super::SagaPhase {
        self.phase
    }

    #[must_use]
    pub const fn cause(&self) -> &FailureCode {
        &self.cause
    }

    /// Forward nodes in the exact path and the forward instances each waits for.
    #[must_use]
    pub const fn entries(&self) -> &BTreeMap<NodeId, Vec<ExecutionInstanceId>> {
        &self.entries
    }

    /// Irreversible forward nodes in the exact path.
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
}

impl PathLoopState {
    /// Restores trusted scalar loop progress.
    ///
    /// # Errors
    /// The counters or active/delay fields contradict the loop phase.
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        kind: LoopKind,
        phase: LoopPhase,
        max_iterations: u32,
        entered_at: time::OffsetDateTime,
        deadline: time::OffsetDateTime,
        initial_carry_digest: Digest,
        current_carry_digest: Digest,
        next_iteration: u32,
        active_iteration: Option<u32>,
        delay_until: Option<time::OffsetDateTime>,
        drain_reason: Option<LoopDrainReason>,
        failure: Option<FailureCode>,
    ) -> Result<Self, ExecutionError> {
        let active_matches = match phase {
            LoopPhase::Running => active_iteration.is_some() && delay_until.is_none(),
            LoopPhase::Delaying => active_iteration.is_none() && delay_until.is_some(),
            LoopPhase::Draining => delay_until.is_none(),
            _ => active_iteration.is_none() && delay_until.is_none(),
        };
        if max_iterations == 0
            || next_iteration > max_iterations
            || active_iteration.is_some_and(|index| index >= max_iterations)
            || !active_matches
        {
            return Err(ExecutionError::Invariant(
                "bounded loop has invalid scalar progress",
            ));
        }
        Ok(Self {
            kind,
            phase,
            max_iterations,
            entered_at,
            deadline,
            initial_carry_digest,
            current_carry_digest,
            next_iteration,
            active_iteration,
            delay_until,
            drain_reason,
            failure,
        })
    }

    fn from_loop(state: &LoopState) -> Result<Self, ExecutionError> {
        Self::restore(
            state.kind(),
            state.phase(),
            state.max_iterations(),
            state.entered_at(),
            state.deadline(),
            state.initial_carry_digest(),
            state.current_carry_digest(),
            state.next_iteration(),
            state.active_iteration(),
            state.delay_until(),
            state.drain_reason().cloned(),
            state.failure().cloned(),
        )
    }

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
    pub const fn entered_at(&self) -> time::OffsetDateTime {
        self.entered_at
    }

    #[must_use]
    pub const fn deadline(&self) -> time::OffsetDateTime {
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
    pub const fn active_iteration(&self) -> Option<u32> {
        self.active_iteration
    }

    #[must_use]
    pub const fn delay_until(&self) -> Option<time::OffsetDateTime> {
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

/// Run-wide values that fence every local transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalExecutionFence {
    run_id: RunId,
    status: RunStatus,
    stop: Option<(StopReason, Option<FailureCode>)>,
    paused: bool,
    version: u64,
    outside_work: super::state::RunWorkSummary,
    compensating: bool,
}

impl GlobalExecutionFence {
    #[must_use]
    pub const fn restore(
        run_id: RunId,
        status: RunStatus,
        stop: Option<(StopReason, Option<FailureCode>)>,
        paused: bool,
        version: u64,
        outside_work: super::state::RunWorkSummary,
        compensating: bool,
    ) -> Self {
        Self {
            run_id,
            status,
            stop,
            paused,
            version,
            outside_work,
            compensating,
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

    #[must_use]
    pub const fn stop_reason(&self) -> Option<StopReason> {
        match &self.stop {
            Some((reason, _)) => Some(*reason),
            None => None,
        }
    }

    #[must_use]
    pub fn stop_code(&self) -> Option<&FailureCode> {
        self.stop.as_ref().and_then(|(_, code)| code.as_ref())
    }

    #[must_use]
    pub const fn is_paused(&self) -> bool {
        self.paused
    }

    #[must_use]
    pub const fn version(&self) -> u64 {
        self.version
    }

    #[must_use]
    pub const fn allows_forward_progress(&self) -> bool {
        !self.paused && self.stop.is_none() && !self.status.is_terminal()
    }
}

/// The owning structured scope of one path segment, without its sibling iteration history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AncestorExecutionFence {
    Collection {
        owner: ExecutionInstanceId,
        iteration_index: u32,
        phase: CollectionPhase,
        active: bool,
    },
    Loop {
        owner: ExecutionInstanceId,
        iteration_index: u32,
        phase: LoopPhase,
        active: bool,
    },
}

impl AncestorExecutionFence {
    #[must_use]
    pub const fn owner(&self) -> &ExecutionInstanceId {
        match self {
            Self::Collection { owner, .. } | Self::Loop { owner, .. } => owner,
        }
    }

    #[must_use]
    pub const fn iteration_index(&self) -> u32 {
        match self {
            Self::Collection {
                iteration_index, ..
            }
            | Self::Loop {
                iteration_index, ..
            } => *iteration_index,
        }
    }

    #[must_use]
    pub const fn collection_phase(&self) -> Option<CollectionPhase> {
        match self {
            Self::Collection { phase, .. } => Some(*phase),
            Self::Loop { .. } => None,
        }
    }

    #[must_use]
    pub const fn loop_phase(&self) -> Option<LoopPhase> {
        match self {
            Self::Collection { .. } => None,
            Self::Loop { phase, .. } => Some(*phase),
        }
    }

    #[must_use]
    pub const fn allows_forward_progress(&self) -> bool {
        match self {
            Self::Collection { phase, active, .. } => {
                matches!(phase, CollectionPhase::Open) && *active
            }
            Self::Loop { phase, active, .. } => matches!(phase, LoopPhase::Running) && *active,
        }
    }
}

/// Complete local state for one exact non-root path and bounded fences outside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathExecutionState {
    global: GlobalExecutionFence,
    path: InstancePath,
    ancestors: Vec<AncestorExecutionFence>,
    steps: BTreeMap<NodeId, StepState>,
    collections: BTreeMap<NodeId, PathCollectionState>,
    loops: BTreeMap<NodeId, PathLoopState>,
    scope_sagas: BTreeMap<ExecutionInstanceId, PathScopeSagaState>,
}

impl PathExecutionState {
    /// Restores a path from trusted exact-path and scalar ancestor projections.
    ///
    /// # Errors
    /// The local path is root, the ancestor count differs from its depth, or an ancestor does not
    /// identify the corresponding owner and iteration in order.
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        global: GlobalExecutionFence,
        path: InstancePath,
        ancestors: Vec<AncestorExecutionFence>,
        steps: BTreeMap<NodeId, StepState>,
        collections: BTreeMap<NodeId, PathCollectionState>,
        loops: BTreeMap<NodeId, PathLoopState>,
        scope_sagas: BTreeMap<ExecutionInstanceId, PathScopeSagaState>,
    ) -> Result<Self, ExecutionError> {
        if path.is_root() || ancestors.len() != path.depth() {
            return Err(ExecutionError::Invariant(
                "bounded path state has incomplete ancestry",
            ));
        }
        let mut parent = InstancePath::root();
        for (segment, ancestor) in path.segments().iter().zip(&ancestors) {
            let expected_owner =
                ExecutionInstanceId::in_path(parent.clone(), segment.scope_node().clone());
            if ancestor.owner() != &expected_owner
                || ancestor.iteration_index() != segment.iteration_index()
            {
                return Err(ExecutionError::Invariant(
                    "bounded path state has mismatched ancestry",
                ));
            }
            parent = parent
                .child(segment.scope_node().clone(), segment.iteration_index())
                .map_err(|_| ExecutionError::Invariant("stored path exceeded its bounds"))?;
        }
        for (owner, saga) in &scope_sagas {
            let ancestor = ancestors.iter().any(|fence| fence.owner() == owner);
            if saga.owner() != owner
                || (!ancestor && owner.path() != &path)
                || saga.entries().values().flatten().any(|after| {
                    after.path().is_root() || after.path().depth() < owner.path().depth() + 1
                })
            {
                return Err(ExecutionError::Invariant(
                    "bounded scope saga has unrelated authority",
                ));
            }
        }
        Ok(Self {
            global,
            path,
            ancestors,
            steps,
            collections,
            loops,
            scope_sagas,
        })
    }

    /// Derives a bounded snapshot from the full replay oracle. Production adapters restore the
    /// same shape directly from exact-path rows and scalar ancestor projections.
    ///
    /// # Errors
    /// The path is root, an ancestor scope is absent, or its admitted iteration does not match the
    /// requested path.
    pub fn from_run(state: &RunState, path: InstancePath) -> Result<Self, ExecutionError> {
        if path.is_root() {
            return Err(ExecutionError::Invariant(
                "bounded path state requires a scoped path",
            ));
        }
        let mut parent = InstancePath::root();
        let mut ancestors = Vec::with_capacity(path.depth());
        for segment in path.segments() {
            let owner = ExecutionInstanceId::in_path(parent.clone(), segment.scope_node().clone());
            let index = segment.iteration_index();
            if let Some(collection) = state.collection(&owner) {
                let active = collection
                    .active_iterations()
                    .any(|candidate| candidate == index);
                if collection.entry_instance(index).is_none() {
                    return Err(ExecutionError::Invariant(
                        "path collection ancestor did not admit its iteration",
                    ));
                }
                ancestors.push(AncestorExecutionFence::Collection {
                    owner,
                    iteration_index: index,
                    phase: collection.phase(),
                    active,
                });
            } else if let Some(loop_state) = state.loop_state(&owner) {
                ancestors.push(AncestorExecutionFence::Loop {
                    owner,
                    iteration_index: index,
                    phase: loop_state.phase(),
                    active: loop_state.active_iteration() == Some(index),
                });
            } else {
                return Err(ExecutionError::Invariant(
                    "path ancestor has no structured scope state",
                ));
            }
            parent = parent
                .child(segment.scope_node().clone(), index)
                .map_err(|_| ExecutionError::Invariant("stored path exceeded its bounds"))?;
        }

        let steps = state
            .instances()
            .filter(|(instance, _)| instance.path() == &path)
            .map(|(instance, step)| (instance.node().clone(), step.clone()))
            .collect();
        let collections = state
            .collections()
            .filter(|(owner, _)| owner.path() == &path)
            .map(|(owner, scope)| {
                PathCollectionState::from_collection(scope)
                    .map(|scalar| (owner.node().clone(), scalar))
            })
            .collect::<Result<_, _>>()?;
        let loops = state
            .loops()
            .filter(|(owner, _)| owner.path() == &path)
            .map(|(owner, scope)| {
                PathLoopState::from_loop(scope).map(|scalar| (owner.node().clone(), scalar))
            })
            .collect::<Result<_, _>>()?;
        let ancestor_owners: std::collections::BTreeSet<_> = ancestors
            .iter()
            .map(|ancestor| ancestor.owner().clone())
            .collect();
        let scope_sagas = state
            .scope_sagas()
            .filter(|(owner, _)| owner.path() == &path || ancestor_owners.contains(*owner))
            .map(|(owner, saga)| {
                let entries = saga
                    .entries()
                    .iter()
                    .filter(|(forward, _)| forward.path() == &path)
                    .map(|(forward, after)| (forward.node().clone(), after.clone()))
                    .collect();
                let irreversible = saga
                    .irreversible()
                    .iter()
                    .filter(|forward| forward.path() == &path)
                    .map(|forward| forward.node().clone())
                    .collect();
                let bounded = PathScopeSagaState::restore(
                    owner.clone(),
                    saga.phase(),
                    saga.cause().clone(),
                    entries,
                    irreversible,
                    saga.outcome(),
                    saga.during_stop(),
                );
                (owner.clone(), bounded)
            })
            .collect();

        Self::restore(
            GlobalExecutionFence::restore(
                state.run_id(),
                state.status(),
                state
                    .stop_reason()
                    .map(|reason| (reason, state.stop_code().cloned())),
                state.is_paused(),
                state.version(),
                super::state::RunWorkSummary::from_steps(
                    state
                        .instances()
                        .filter(|(instance, _)| instance.path() != &path)
                        .map(|(_, step)| step),
                ),
                state.compensation_in_progress(),
            ),
            path,
            ancestors,
            steps,
            collections,
            loops,
            scope_sagas,
        )
    }

    #[must_use]
    pub const fn global(&self) -> &GlobalExecutionFence {
        &self.global
    }

    #[must_use]
    pub const fn path(&self) -> &InstancePath {
        &self.path
    }

    #[must_use]
    pub fn ancestors(&self) -> &[AncestorExecutionFence] {
        &self.ancestors
    }

    #[must_use]
    pub fn local_instance(&self, node: &NodeId) -> Option<&StepState> {
        self.steps.get(node)
    }

    pub fn local_instances(&self) -> impl Iterator<Item = (&NodeId, &StepState)> {
        self.steps.iter()
    }

    #[must_use]
    pub fn local_collection(&self, node: &NodeId) -> Option<&PathCollectionState> {
        self.collections.get(node)
    }

    #[must_use]
    pub fn local_loop(&self, node: &NodeId) -> Option<&PathLoopState> {
        self.loops.get(node)
    }

    #[must_use]
    pub fn scope_saga(&self, owner: &ExecutionInstanceId) -> Option<&PathScopeSagaState> {
        self.scope_sagas.get(owner)
    }

    pub fn scope_sagas(&self) -> impl Iterator<Item = (&ExecutionInstanceId, &PathScopeSagaState)> {
        self.scope_sagas.iter()
    }

    /// Whether the exact path's owning scope durably planned this compensation.
    #[must_use]
    pub fn authorizes_scope_compensation(&self, instance: &ExecutionInstanceId) -> bool {
        if instance.path() != &self.path || !instance.node().is_compensation() {
            return false;
        }
        let Some(owner) = instance.path().scope_owner() else {
            return false;
        };
        let Some(forward) = instance.node().forward() else {
            return false;
        };
        self.scope_sagas.get(&owner).is_some_and(|saga| {
            saga.phase() == super::SagaPhase::Compensating && saga.entries().contains_key(&forward)
        })
    }

    /// Whether a persisted local forward step durably requested this reconciliation.
    #[must_use]
    pub fn authorizes_reconciliation(&self, instance: &ExecutionInstanceId) -> bool {
        if instance.path() != &self.path || !instance.node().is_reconciliation() {
            return false;
        }
        let Some(forward) = instance.node().forward() else {
            return false;
        };
        self.steps.get(&forward).is_some_and(|step| {
            step.status() == StepStatus::Waiting
                && matches!(step.waiting_on(), Some(super::WaitingOn::Reconciliation))
        })
    }

    /// Whether this exact bounded projection contains the durable origin of derived work.
    #[must_use]
    pub fn authorizes_derived_work(&self, instance: &ExecutionInstanceId) -> bool {
        match (
            instance.node().is_compensation(),
            instance.node().is_reconciliation(),
        ) {
            (true, false) => self.authorizes_scope_compensation(instance),
            (false, true) => self.authorizes_reconciliation(instance),
            _ => false,
        }
    }

    /// Applies one exact-path worker lifecycle or routing event with the same step reducer used by
    /// the full replay oracle.
    ///
    /// # Errors
    /// The event addresses another path, the run is terminal, or the local transition is illegal.
    pub fn apply_addressed(&mut self, addressed: &AddressedRunEvent) -> Result<(), ExecutionError> {
        let instance = addressed.instance().ok_or(ExecutionError::Invariant(
            "a bounded event has no execution instance",
        ))?;
        if addressed.path() != &self.path {
            return Err(ExecutionError::UnknownInstance(instance));
        }
        if self.global.status.is_terminal() {
            return Err(ExecutionError::RunFinished(self.global.status));
        }
        let next_version = self
            .global
            .version
            .checked_add(1)
            .ok_or(ExecutionError::Invariant("the run version overflowed"))?;
        let name = addressed.event().name();
        match addressed.event() {
            RunEvent::StepRouted { activated, .. } => {
                let step = self
                    .steps
                    .get(instance.node())
                    .ok_or_else(|| ExecutionError::UnknownInstance(instance.clone()))?;
                if !matches!(
                    step.status(),
                    StepStatus::Succeeded | StepStatus::Failed | StepStatus::Skipped
                ) || step.routed()
                {
                    return Err(ExecutionError::IllegalInstanceTransition {
                        instance,
                        status: step.status(),
                        event: name,
                    });
                }
                for (position, node) in activated.iter().enumerate() {
                    if self.steps.contains_key(node) || activated[..position].contains(node) {
                        return Err(ExecutionError::AlreadyScheduled(node.clone()));
                    }
                }
                for node in activated {
                    self.steps.insert(node.clone(), StepState::pending());
                }
                self.steps
                    .get_mut(instance.node())
                    .ok_or(ExecutionError::Invariant(
                        "validated bounded step disappeared",
                    ))?
                    .mark_routed();
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
                    .get_mut(instance.node())
                    .ok_or_else(|| ExecutionError::UnknownInstance(instance.clone()))?
                    .apply_instance_lifecycle(&instance, event)?;
            }
            event @ (RunEvent::ReconciliationRequested { .. }
            | RunEvent::ReconciliationConcluded { .. }
            | RunEvent::StepNeedsReview { .. }
            | RunEvent::StepRetryRequested { .. }
            | RunEvent::StepSkipped { .. }
            | RunEvent::StepResolved { .. }) => {
                super::state::effects::apply_effect_event(self, &instance, event)?;
            }
            _ => return Err(ExecutionError::UnsupportedScopedEvent(name)),
        }
        self.global.status = self
            .global
            .outside_work
            .union(super::state::RunWorkSummary::from_steps(
                self.steps.values(),
            ))
            .status(
                self.global.compensating,
                self.global.stop.is_some(),
                self.global.paused,
            );
        self.global.version = next_version;
        Ok(())
    }

    #[must_use]
    pub fn allows_forward_progress(&self) -> bool {
        self.global.allows_forward_progress()
            && self
                .ancestors
                .iter()
                .all(AncestorExecutionFence::allows_forward_progress)
    }
}

impl super::state::effects::InstanceSteps for PathExecutionState {
    fn get_effect_step(&self, instance: &ExecutionInstanceId) -> Option<&StepState> {
        (instance.path() == &self.path)
            .then(|| self.steps.get(instance.node()))
            .flatten()
    }
    fn get_effect_step_mut(&mut self, instance: &ExecutionInstanceId) -> Option<&mut StepState> {
        if instance.path() != &self.path {
            return None;
        }
        self.steps.get_mut(instance.node())
    }
    fn reconciliation_step_mut(
        &mut self,
        instance: &ExecutionInstanceId,
    ) -> Result<&mut StepState, ExecutionError> {
        if instance.path() != &self.path {
            return Err(ExecutionError::UnknownInstance(instance.clone()));
        }
        Ok(self
            .steps
            .entry(instance.node().clone())
            .or_insert_with(StepState::pending))
    }
}
