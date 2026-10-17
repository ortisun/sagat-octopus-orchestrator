//! What the planner does about the compensation of a collection or loop scope (feature 014).
//!
//! A scope whose body can be compensated ([`WorkflowDefinition::scope_compensates`]) is its own
//! saga. Its drain is the scope's own business (`collection.drain_started`, `loop.drain_started`
//! and the iteration drains); this module starts where the drain ends:
//!
//! 1. **Plan** — once every iteration has settled, one event records which completed members are
//!    to be undone and which compensations each of them waits for: reverse topological inside an
//!    iteration, nothing across the iterations of a collection, last iteration first across the
//!    iterations of a loop. A member whose last attempt ended with an unknown outcome is undone
//!    defensively; a member that cannot be undone is recorded, so the scope cannot end
//!    `COMPENSATED`.
//! 2. **Compensate** — each compensation is released when everything it waits for has finished.
//!    A collection releases at most `maxConcurrency` iterations at a time. A compensation that
//!    keeps failing waits for a person and is never given up on by a stop of the run.
//! 3. **Done** — `scope.compensated` with the outcome. The store then finishes the scope.
//!
//! All of it is a pure function of the definition and the state; the events are the only record.

use std::collections::{BTreeMap, BTreeSet};

use super::error::ExecutionError;
use super::event::{
    AddressedRunEvent, CollectionDrainReason, FailureCode, LoopDrainReason, RunEvent,
    ScopedPlannedEntry,
};
use super::instance::{ExecutionInstanceId, InstancePath};
use super::plan::AddressedPlan;
use super::state::{
    CollectionPhase, LoopPhase, RunState, SagaPhase, ScopeSagaState, StepState, StepStatus,
};
use crate::saga::{NodeOrigin, reverse_dependencies};
use crate::workflow::{NodeId, WorkflowDefinition};

/// The next thing the compensating scopes of the run need, if there is one.
///
/// # Errors
/// [`ExecutionError`] when the state names a scope or step the definition does not have (the state
/// belongs to another version).
pub fn plan_scope_compensation(
    def: &WorkflowDefinition,
    state: &RunState,
) -> Result<AddressedPlan, ExecutionError> {
    let owners: BTreeSet<ExecutionInstanceId> = state
        .collections()
        .map(|(owner, _)| owner.clone())
        .chain(state.loops().map(|(owner, _)| owner.clone()))
        .collect();
    for owner in &owners {
        if !def.scope_compensates(owner) {
            continue;
        }
        let event = match state.scope_saga(owner) {
            None => plan_compensation(def, state, owner)?,
            Some(saga) if saga.phase() == SagaPhase::Compensating => {
                next_compensation(state, owner, saga)?
            }
            Some(_) => None,
        };
        if let Some(event) = event {
            return Ok(AddressedPlan::Emit(event));
        }
    }
    Ok(AddressedPlan::Idle)
}

/// Whether the scope's drain has settled every iteration and nothing is left to wait for.
fn drained(state: &RunState, owner: &ExecutionInstanceId) -> bool {
    if let Some(collection) = state.collection(owner) {
        return collection.phase() == CollectionPhase::Draining && collection.active_count() == 0;
    }
    state.loop_state(owner).is_some_and(|scope| {
        scope.phase() == LoopPhase::Draining && scope.active_iteration().is_none()
    })
}

/// The failure that is the reason the scope compensates.
fn cause(state: &RunState, owner: &ExecutionInstanceId) -> FailureCode {
    if let Some(collection) = state.collection(owner) {
        return match collection.drain_reason() {
            Some(CollectionDrainReason::Failed { code, .. }) => code.clone(),
            Some(CollectionDrainReason::StorageQuotaExceeded) => {
                FailureCode::storage_quota_exceeded()
            }
            Some(CollectionDrainReason::Cancelled) => FailureCode::run_cancelled(),
            None => FailureCode::branch_failed(),
        };
    }
    state
        .loop_state(owner)
        .and_then(|scope| scope.drain_reason())
        .map_or_else(FailureCode::branch_failed, |reason| match reason {
            LoopDrainReason::Failed { code, .. } => code.clone(),
            LoopDrainReason::TimedOut => FailureCode::run_timed_out(),
            LoopDrainReason::Cancelled => FailureCode::run_cancelled(),
        })
}

fn is_loop(state: &RunState, owner: &ExecutionInstanceId) -> bool {
    state.loop_state(owner).is_some()
}

/// The last segment of a scoped path: the iteration index, when the path is directly under `owner`.
fn iteration_of(owner: &ExecutionInstanceId, path: &InstancePath) -> Option<u32> {
    let (last, parents) = path.segments().split_last()?;
    (last.scope_node() == owner.node() && parents == owner.path().segments())
        .then(|| last.iteration_index())
}

fn addressed(path: &InstancePath, event: RunEvent) -> Result<AddressedRunEvent, ExecutionError> {
    AddressedRunEvent::in_path(path.clone(), event)
        .map_err(|_| ExecutionError::Invariant("a scope event lost its instance path"))
}

/// What one iteration contributes to the plan, keyed by the iteration index.
#[derive(Default)]
struct Iteration {
    /// Completed members, or members whose outcome is unknown, that have a compensation.
    compensable: BTreeSet<NodeId>,
    /// Members that completed and cannot be undone.
    irreversible: Vec<NodeId>,
}

fn plan_compensation(
    def: &WorkflowDefinition,
    state: &RunState,
    owner: &ExecutionInstanceId,
) -> Result<Option<AddressedRunEvent>, ExecutionError> {
    if !drained(state, owner) {
        return Ok(None);
    }
    let body = def
        .scope_body(owner)
        .ok_or_else(|| ExecutionError::UnknownInstance(owner.clone()))?;
    // 1. What each iteration completed, from the instances themselves.
    let mut iterations: BTreeMap<u32, (InstancePath, Iteration)> = BTreeMap::new();
    for (instance, step) in state.instances() {
        let Some(index) = iteration_of(owner, instance.path()) else {
            continue;
        };
        let Some(node) = body.node(instance.node()) else {
            continue;
        };
        if node.origin() != &NodeOrigin::Declared {
            continue;
        }
        let entry = iterations
            .entry(index)
            .or_insert_with(|| (instance.path().clone(), Iteration::default()));
        let completed = step.status() == StepStatus::Succeeded;
        let unknown = matches!(step.status(), StepStatus::Cancelled | StepStatus::Failed)
            && step
                .failure()
                .is_some_and(FailureCode::leaves_outcome_unknown);
        if !(completed || unknown) {
            continue;
        }
        let compensable = body.node(&instance.node().compensation()).is_some()
            && state.instance(&instance.compensation()).is_none();
        if compensable {
            entry.1.compensable.insert(instance.node().clone());
        }
        if node.effect().irreversible || (unknown && !compensable && node.effect().high_risk) {
            entry.1.irreversible.push(instance.node().clone());
        }
    }

    // 2. Reverse topological order inside each iteration.
    let mut entries: BTreeMap<ExecutionInstanceId, Vec<ExecutionInstanceId>> = BTreeMap::new();
    let mut by_iteration: BTreeMap<u32, Vec<ExecutionInstanceId>> = BTreeMap::new();
    let mut irreversible: Vec<ExecutionInstanceId> = Vec::new();
    for (index, (path, iteration)) in &iterations {
        let deps = reverse_dependencies(body, &iteration.compensable);
        for node in &iteration.compensable {
            let instance = ExecutionInstanceId::in_path(path.clone(), node.clone());
            let after = deps
                .get(node)
                .into_iter()
                .flatten()
                .map(|earlier| ExecutionInstanceId::in_path(path.clone(), earlier.clone()))
                .collect();
            by_iteration
                .entry(*index)
                .or_default()
                .push(instance.clone());
            entries.insert(instance, after);
        }
        irreversible.extend(
            iteration
                .irreversible
                .iter()
                .map(|node| ExecutionInstanceId::in_path(path.clone(), node.clone())),
        );
    }

    // 3. Across iterations: none for a collection (they ran concurrently); last first for a loop,
    // because iteration k+1 depends on the carry of k. An entry that starts an iteration's
    // compensation waits for the entries that finish the nearest later iteration's, which in turn
    // wait for everything else of that iteration, so the links stay linear in the entries.
    if is_loop(state, owner) {
        let mut later_last: Option<Vec<ExecutionInstanceId>> = None;
        for (_, members) in by_iteration.iter().rev() {
            let waited_for: BTreeSet<&ExecutionInstanceId> = members
                .iter()
                .flat_map(|member| entries.get(member).into_iter().flatten())
                .collect();
            let last: Vec<ExecutionInstanceId> = members
                .iter()
                .filter(|member| !waited_for.contains(member))
                .cloned()
                .collect();
            if let Some(later) = &later_last {
                for member in members {
                    if let Some(after) = entries.get_mut(member)
                        && after.is_empty()
                    {
                        after.extend(later.iter().cloned());
                    }
                }
            }
            later_last = Some(last);
        }
    }

    irreversible.sort();
    let planned: Vec<ScopedPlannedEntry> = entries
        .into_iter()
        .map(|(instance, mut after)| {
            after.sort();
            after.dedup();
            ScopedPlannedEntry { instance, after }
        })
        .collect();
    Ok(Some(addressed(
        owner.path(),
        RunEvent::ScopeCompensationPlanned {
            node: owner.node().clone(),
            cause: cause(state, owner),
            entries: planned,
            irreversible,
        },
    )?))
}

fn status_of(state: &RunState, forward: &ExecutionInstanceId) -> Option<StepStatus> {
    state
        .instance(&forward.compensation())
        .map(StepState::status)
}

fn next_compensation(
    state: &RunState,
    owner: &ExecutionInstanceId,
    saga: &ScopeSagaState,
) -> Result<Option<AddressedRunEvent>, ExecutionError> {
    let broken = |forward: &ExecutionInstanceId| {
        matches!(
            status_of(state, forward),
            Some(StepStatus::Failed | StepStatus::Cancelled)
        )
    };
    // What waits for a compensation that will not happen cannot be released either: it is given up
    // on, and the scope ends as failed. (A person's decision is the only thing that ends a wait for
    // a person; a stop of the run does not.)
    for (forward, after) in saga.entries() {
        if status_of(state, forward) == Some(StepStatus::Pending) && after.iter().any(&broken) {
            let compensation = forward.compensation();
            return Ok(Some(addressed(
                compensation.path(),
                RunEvent::StepCancelled {
                    node: compensation.node().clone(),
                },
            )?));
        }
    }
    // A collection bounds how many iterations compensate at once by its own concurrency; an
    // iteration that has begun is always allowed to go on, so a started iteration is finished
    // before another one is started.
    let cap = state
        .collection(owner)
        .map(|collection| usize::from(collection.max_concurrency()));
    let mut open: BTreeSet<u32> = BTreeSet::new();
    let mut touched: BTreeMap<u32, (bool, bool)> = BTreeMap::new();
    for forward in saga.entries().keys() {
        let Some(index) = iteration_of(owner, forward.path()) else {
            continue;
        };
        let status = status_of(state, forward);
        let flag = touched.entry(index).or_default();
        flag.0 |= !matches!(status, Some(StepStatus::Pending) | None);
        flag.1 |= !status.is_some_and(StepStatus::is_terminal);
    }
    for (index, (started, unfinished)) in &touched {
        if *started && *unfinished {
            open.insert(*index);
        }
    }
    let mut ready: Vec<(bool, u32, &ExecutionInstanceId)> = Vec::new();
    for (forward, after) in saga.entries() {
        if status_of(state, forward) != Some(StepStatus::Pending) {
            continue;
        }
        let waits = after.iter().all(|earlier| {
            matches!(
                status_of(state, earlier),
                Some(StepStatus::Succeeded | StepStatus::Skipped)
            )
        });
        if !waits {
            continue;
        }
        let index = iteration_of(owner, forward.path()).unwrap_or(0);
        ready.push((!open.contains(&index), index, forward));
    }
    ready.sort();
    for (starts_new, _, forward) in ready {
        if starts_new && cap.is_some_and(|cap| open.len() >= cap) {
            continue;
        }
        let compensation = forward.compensation();
        return Ok(Some(addressed(
            compensation.path(),
            RunEvent::StepReleased {
                node: compensation.node().clone(),
            },
        )?));
    }
    let finished = saga
        .entries()
        .keys()
        .all(|forward| status_of(state, forward).is_some_and(StepStatus::is_terminal));
    if finished {
        let outcome = state.expected_scope_outcome(saga);
        return Ok(Some(addressed(
            owner.path(),
            RunEvent::ScopeCompensated {
                node: owner.node().clone(),
                outcome,
            },
        )?));
    }
    Ok(None)
}
