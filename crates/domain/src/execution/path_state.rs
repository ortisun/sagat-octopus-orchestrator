//! Bounded state for one admitted execution path.
//!
//! The complete [`RunState`](super::RunState) remains the replay and verification oracle. Runtime
//! progress can use this projection because it owns every step and nested scope in one exact path,
//! plus typed summaries of each ancestor and the global run fences that can stop local progress.

use std::collections::BTreeMap;

use super::{
    CollectionPhase, CollectionState, ExecutionError, ExecutionInstanceId, FailureCode,
    InstancePath, LoopPhase, LoopState, RunId, RunState, RunStatus, ScopeSagaState, StepState,
    StopReason,
};
use crate::workflow::NodeId;

/// Run-wide values that fence every local transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalExecutionFence {
    run_id: RunId,
    status: RunStatus,
    stop: Option<(StopReason, Option<FailureCode>)>,
    paused: bool,
    version: u64,
}

impl GlobalExecutionFence {
    #[must_use]
    pub const fn restore(
        run_id: RunId,
        status: RunStatus,
        stop: Option<(StopReason, Option<FailureCode>)>,
        paused: bool,
        version: u64,
    ) -> Self {
        Self {
            run_id,
            status,
            stop,
            paused,
            version,
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
    collections: BTreeMap<NodeId, CollectionState>,
    loops: BTreeMap<NodeId, LoopState>,
    scope_sagas: BTreeMap<NodeId, ScopeSagaState>,
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
        collections: BTreeMap<NodeId, CollectionState>,
        loops: BTreeMap<NodeId, LoopState>,
        scope_sagas: BTreeMap<NodeId, ScopeSagaState>,
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
            .map(|(owner, scope)| (owner.node().clone(), scope.clone()))
            .collect();
        let loops = state
            .loops()
            .filter(|(owner, _)| owner.path() == &path)
            .map(|(owner, scope)| (owner.node().clone(), scope.clone()))
            .collect();
        let scope_sagas = state
            .scope_sagas()
            .filter(|(owner, _)| owner.path() == &path)
            .map(|(owner, saga)| (owner.node().clone(), saga.clone()))
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
    pub fn local_collection(&self, node: &NodeId) -> Option<&CollectionState> {
        self.collections.get(node)
    }

    #[must_use]
    pub fn local_loop(&self, node: &NodeId) -> Option<&LoopState> {
        self.loops.get(node)
    }

    #[must_use]
    pub fn local_scope_saga(&self, node: &NodeId) -> Option<&ScopeSagaState> {
        self.scope_sagas.get(node)
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
