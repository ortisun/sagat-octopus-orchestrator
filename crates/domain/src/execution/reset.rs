//! Reset to a point (feature 018, US06, `FR-018-035`..`FR-018-037`): which events a reset may carry over,
//! where a run may be reset, and what the reset would restore and run again.
//!
//! Pure: the journal of a source run goes in, the eligible points and the plan come out. The registry is the
//! normative one of `specs/018-schedules-bulk-operations/contracts/journal-and-events.md` (decision D1605):
//! every journal event type has exactly one rule, and a point whose prefix holds a type without one is not
//! offered.

use std::collections::BTreeSet;

use super::{
    AddressedRunEvent, CollectionPhase, ExecutionError, ExecutionInstanceId, LoopPhase, RunEvent,
    RunId, RunState, RunStatus, StepStatus,
};
use crate::workflow::{NodeId, NodeKind, WorkflowDefinition};

/// What a reset does with an event type of the prefix it re-emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetRule {
    /// Re-emitted under the new run, marked `restored`, and nothing else.
    Copy,
    /// Re-emitted, and the reset re-creates the side-table rows the event implies for the new run.
    Rematerialize,
    /// Must not be in a prefix: every point at or after it is ineligible.
    Exclude,
    /// Opens a window: the points after it are ineligible until the event named in `until` closes it
    /// (the same step); with none named, until the end.
    Refuse { until: Option<&'static str> },
}

/// Declares the rule of every journal event type once, as both an exhaustive `match` over the event (a new
/// event type without a rule does not compile) and a table keyed by the name the journal stores (the way a
/// type written by a version this one does not know is looked up).
macro_rules! registry {
    ($($pattern:pat => ($name:literal, $rule:expr)),+ $(,)?) => {
        /// The rule of an event.
        #[must_use]
        pub fn rule_of(event: &RunEvent) -> ResetRule {
            match event {
                $($pattern => $rule),+
            }
        }

        /// Every event type with its rule, by the name the journal stores.
        pub const RULES: &[(&str, ResetRule)] = &[$(($name, $rule)),+];
    };
}

registry! {
    RunEvent::RunStarted { .. } => ("run.started", ResetRule::Copy),
    RunEvent::ExecutionChainStarted { .. } => ("execution_chain.started", ResetRule::Exclude),
    RunEvent::ExecutionChainSuccessorLinked { .. } => ("execution_chain.successor_linked", ResetRule::Exclude),
    RunEvent::StepRouted { .. } => ("step.routed", ResetRule::Copy),
    RunEvent::StepReleased { .. } => ("step.released", ResetRule::Copy),
    RunEvent::StepLeased { .. } => ("step.leased", ResetRule::Copy),
    RunEvent::StepStarted { .. } => ("step.started", ResetRule::Copy),
    RunEvent::StepCompleted { .. } => ("step.completed", ResetRule::Copy),
    RunEvent::StepRetryScheduled { .. } => ("step.retry_scheduled", ResetRule::Copy),
    RunEvent::StepRetryDue { .. } => ("step.retry_due", ResetRule::Copy),
    RunEvent::StepFailed { .. } => ("step.failed", ResetRule::Copy),
    RunEvent::ConditionEvaluated { .. } => ("condition.evaluated", ResetRule::Copy),
    RunEvent::SwitchEvaluated { .. } => ("switch.evaluated", ResetRule::Copy),
    RunEvent::TimerStarted { .. } => ("timer.started", ResetRule::Copy),
    RunEvent::TimerFired { .. } => ("timer.fired", ResetRule::Copy),
    RunEvent::WaitStarted { .. } => ("wait.started", ResetRule::Copy),
    RunEvent::WaitSatisfied { .. } => ("wait.satisfied", ResetRule::Copy),
    RunEvent::WaitTimedOut { .. } => ("wait.timed_out", ResetRule::Copy),
    RunEvent::ApprovalOpened { .. } => ("approval.opened", ResetRule::Copy),
    RunEvent::ApprovalDecided { .. } => ("approval.decided", ResetRule::Copy),
    RunEvent::ApprovalExpired { .. } => ("approval.expired", ResetRule::Copy),
    RunEvent::ParallelForked { .. } => ("parallel.forked", ResetRule::Copy),
    RunEvent::ParallelJoined { .. } => ("parallel.joined", ResetRule::Copy),
    RunEvent::CollectionEntered { .. } => ("collection.entered", ResetRule::Copy),
    RunEvent::CollectionIterationAdmitted { .. } => ("collection.iteration_admitted", ResetRule::Copy),
    RunEvent::CollectionIterationReturned { .. } => ("collection.iteration_returned", ResetRule::Copy),
    RunEvent::CollectionIterationCheckpointed { .. } => ("collection.iteration_checkpointed", ResetRule::Copy),
    RunEvent::CollectionDrainStarted { .. } => ("collection.drain_started", ResetRule::Copy),
    RunEvent::CollectionIterationDrained { .. } => ("collection.iteration_drained", ResetRule::Copy),
    RunEvent::CollectionFinished { .. } => ("collection.finished", ResetRule::Copy),
    RunEvent::LoopEntered { .. } => ("loop.entered", ResetRule::Copy),
    RunEvent::LoopIterationAdmitted { .. } => ("loop.iteration_admitted", ResetRule::Copy),
    RunEvent::LoopIterationReturned { .. } => ("loop.iteration_returned", ResetRule::Copy),
    RunEvent::LoopIterationCheckpointed { .. } => ("loop.iteration_checkpointed", ResetRule::Copy),
    RunEvent::LoopDelayElapsed { .. } => ("loop.delay_elapsed", ResetRule::Copy),
    RunEvent::LoopDrainStarted { .. } => ("loop.drain_started", ResetRule::Copy),
    RunEvent::LoopIterationDrained { .. } => ("loop.iteration_drained", ResetRule::Copy),
    RunEvent::LoopFinished { .. } => ("loop.finished", ResetRule::Copy),
    RunEvent::ChildStarted { .. } => ("child.started", ResetRule::Copy),
    RunEvent::ChildFinished { .. } => ("child.finished", ResetRule::Copy),
    RunEvent::EndReached { .. } => ("end.reached", ResetRule::Exclude),
    RunEvent::StepCancelRequested { .. } => ("step.cancel_requested", ResetRule::Copy),
    RunEvent::StepCancelled { .. } => ("step.cancelled", ResetRule::Copy),
    RunEvent::SagaEntered { .. } => ("saga.entered", ResetRule::Copy),
    RunEvent::SagaCompleted { .. } => ("saga.completed", ResetRule::Copy),
    RunEvent::SagaAborting { .. } => ("saga.aborting", ResetRule::Exclude),
    RunEvent::CompensationPlanned { .. } => ("compensation.planned", ResetRule::Exclude),
    RunEvent::SagaCompensated { .. } => ("saga.compensated", ResetRule::Exclude),
    RunEvent::ScopeCompensationPlanned { .. } => ("scope.compensation_planned", ResetRule::Exclude),
    RunEvent::ScopeCompensated { .. } => ("scope.compensated", ResetRule::Exclude),
    RunEvent::ReconciliationRequested { .. } => ("reconciliation.requested", ResetRule::Refuse { until: Some("reconciliation.concluded") }),
    RunEvent::ReconciliationConcluded { .. } => ("reconciliation.concluded", ResetRule::Copy),
    RunEvent::StepNeedsReview { .. } => ("step.needs_review", ResetRule::Copy),
    RunEvent::StepRetryRequested { .. } => ("step.retry_requested", ResetRule::Copy),
    RunEvent::StepSkipped { .. } => ("step.skipped", ResetRule::Copy),
    RunEvent::StepResolved { .. } => ("step.resolved", ResetRule::Copy),
    RunEvent::StateUpdated { .. } => ("state.updated", ResetRule::Copy),
    RunEvent::RunContinued { .. } => ("run.continued", ResetRule::Exclude),
    RunEvent::ContinuationSelected { .. } => ("continuation.selected", ResetRule::Exclude),
    RunEvent::RunPaused => ("run.paused", ResetRule::Copy),
    RunEvent::RunResumed => ("run.resumed", ResetRule::Copy),
    RunEvent::StartDelayed { .. } => ("run.start_delayed", ResetRule::Refuse { until: Some("run.start_released") }),
    RunEvent::StartReleased { .. } => ("run.start_released", ResetRule::Copy),
    RunEvent::RunReset { .. } => ("run.reset", ResetRule::Copy),
    RunEvent::RunStopRequested { .. } => ("run.stop_requested", ResetRule::Exclude),
    RunEvent::RunFinished { .. } => ("run.finished", ResetRule::Exclude),
}

/// The rule of an event type by the name the journal stores, if this version knows the type. A journal row
/// of a type it does not know has no rule: a reset of a prefix that holds one is refused (`RESET_RULE_MISSING`).
#[must_use]
pub fn rule_by_name(name: &str) -> Option<ResetRule> {
    RULES
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(_, rule)| *rule)
}

/// The events that end a root node and so are the points a run can be reset to, besides its start (D1022).
const COMPLETIONS: [&str; 4] = [
    "step.completed",
    "child.finished",
    "collection.finished",
    "loop.finished",
];

/// One place in the journal a run may be reset to: the sequence of the last event it keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetPoint {
    /// `N`: the events `1..=N` are re-emitted.
    pub sequence: u64,
    /// The root node whose completion this is; none for the start of the run.
    pub completes: Option<NodeId>,
}

/// Why a candidate point is not offered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResetBlock {
    /// An event type with rule `EXCLUDE` is in the prefix (a terminal tail, a compensation, a chain).
    ExcludedEvent(&'static str),
    /// A window opened by an event with rule `REFUSE` is still open at the point.
    OpenWindow(&'static str),
    /// The run is not plainly running at the point (waiting, paused, stopping, compensating, in review).
    NotRunning(RunStatus),
    /// A step is not finished at the point: an attempt in flight, a wait, a timer, a retry, a branch.
    OpenWork(NodeId),
    /// A collection or a loop has not finished successfully.
    ActiveScope,
    /// An instance of the prefix is compensated, skipped, resolved or reconciled later in the source, so a
    /// reset run, which would not have had that happen, cannot inherit it.
    ChangedLater {
        event_type: &'static str,
        node: NodeId,
    },
}

/// The candidate points of a journal and what became of each.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResetPoints {
    pub eligible: Vec<ResetPoint>,
    pub blocked: Vec<(ResetPoint, ResetBlock)>,
}

/// A window an event with rule `REFUSE` opened.
struct Window {
    opened: &'static str,
    until: Option<&'static str>,
    node: Option<NodeId>,
}

/// A candidate while the journal is read: its facts at the point.
struct Candidate {
    point: ResetPoint,
    block: Option<ResetBlock>,
    nodes: BTreeSet<NodeId>,
    instances: BTreeSet<ExecutionInstanceId>,
}

const fn is_open(status: StepStatus) -> bool {
    matches!(
        status,
        StepStatus::Pending
            | StepStatus::Ready
            | StepStatus::Leased
            | StepStatus::Running
            | StepStatus::Waiting
            | StepStatus::RetryWait
    )
}

/// What the replayed state says about the point, if it is not quiet. `start` is the point right after
/// `run.started`, where the entry step exists and has not been released yet: that is the clean start of the
/// run, not work in flight.
fn state_block(state: &RunState, start: bool) -> Option<ResetBlock> {
    if state.status() != RunStatus::Running || state.is_paused() || state.stop_reason().is_some() {
        return Some(ResetBlock::NotRunning(state.status()));
    }
    if !start
        && let Some((instance, _)) = state.instances().find(|(_, step)| is_open(step.status()))
    {
        return Some(ResetBlock::OpenWork(instance.node().clone()));
    }
    if state
        .collections()
        .any(|(_, scope)| scope.phase() != CollectionPhase::Succeeded)
        || state
            .loops()
            .any(|(_, scope)| scope.phase() != LoopPhase::Succeeded)
    {
        return Some(ResetBlock::ActiveScope);
    }
    None
}

/// The forward node an event about a node concerns: a compensation or a reconciliation is about the node it
/// derives from.
fn forward_of(node: &NodeId) -> NodeId {
    node.forward().unwrap_or_else(|| node.clone())
}

/// The prefix instance a later event compensates, skips, resolves or reconciles, if it does.
fn touched(
    event: &AddressedRunEvent,
    nodes: &BTreeSet<NodeId>,
    instances: &BTreeSet<ExecutionInstanceId>,
) -> Option<NodeId> {
    let node_in_prefix = |node: &NodeId| {
        let node = forward_of(node);
        nodes.contains(&node).then_some(node)
    };
    match event.event() {
        RunEvent::StepSkipped { node }
        | RunEvent::StepResolved { node, .. }
        | RunEvent::StepNeedsReview { node, .. }
        | RunEvent::ReconciliationRequested { node, .. }
        | RunEvent::ReconciliationConcluded { node, .. } => {
            if event.path().is_root() {
                node_in_prefix(node)
            } else {
                let instance = event.instance()?;
                let forward =
                    ExecutionInstanceId::in_path(instance.path().clone(), forward_of(node));
                instances.contains(&forward).then(|| forward_of(node))
            }
        }
        RunEvent::CompensationPlanned {
            entries,
            irreversible,
            ..
        } => entries
            .iter()
            .map(|entry| &entry.node)
            .chain(irreversible)
            .find_map(node_in_prefix),
        RunEvent::ScopeCompensationPlanned {
            entries,
            irreversible,
            ..
        } => entries
            .iter()
            .map(|entry| &entry.instance)
            .chain(irreversible)
            .find(|instance| instances.contains(instance))
            .map(|instance| instance.node().clone()),
        _ => None,
    }
}

/// Where a run may be reset to (D1022).
///
/// A candidate is `run.started` and the completion of a root node (`step.completed`, `child.finished`,
/// `collection.finished`, `loop.finished`). It is eligible when no event type in the events up to it has the
/// rule `EXCLUDE` or an open `REFUSE` window; the replayed state at it is quiet (running, not paused or
/// stopping, no step open, every scope finished); and nothing later in the source compensates, skips,
/// resolves or reconciles an instance that already exists at it. The rule of an event type this version does
/// not know cannot be looked up here, because the journal could not be decoded: the store refuses the reset
/// with the type's name (`RESET_RULE_MISSING`, see [`rule_by_name`]).
///
/// # Errors
/// The first event the reducer rejects: a journal that does not replay has no points.
pub fn reset_points(
    run_id: RunId,
    journal: &[AddressedRunEvent],
) -> Result<ResetPoints, ExecutionError> {
    let mut state = RunState::new(run_id);
    let mut excluded: Option<&'static str> = None;
    let mut windows: Vec<Window> = Vec::new();
    let mut candidates: Vec<Candidate> = Vec::new();
    for (index, addressed) in journal.iter().enumerate() {
        state.apply_addressed(addressed)?;
        let event = addressed.event();
        match rule_of(event) {
            ResetRule::Exclude => {
                excluded.get_or_insert(event.name());
            }
            ResetRule::Refuse { until } => windows.push(Window {
                opened: event.name(),
                until,
                node: event.node().cloned(),
            }),
            ResetRule::Copy | ResetRule::Rematerialize => {}
        }
        windows.retain(|window| {
            !(window.until == Some(event.name()) && window.node.as_ref() == event.node())
        });
        let name = event.name();
        let point =
            addressed.path().is_root() && (name == "run.started" || COMPLETIONS.contains(&name));
        if !point {
            continue;
        }
        let block = excluded
            .map(ResetBlock::ExcludedEvent)
            .or_else(|| {
                windows
                    .first()
                    .map(|window| ResetBlock::OpenWindow(window.opened))
            })
            .or_else(|| state_block(&state, name == "run.started"));
        candidates.push(Candidate {
            point: ResetPoint {
                sequence: u64::try_from(index + 1).map_err(|_| {
                    ExecutionError::Invariant("a journal is shorter than 2^64 events")
                })?,
                completes: (name != "run.started")
                    .then(|| event.node().cloned())
                    .flatten(),
            },
            block,
            nodes: state.steps().map(|(node, _)| node.clone()).collect(),
            instances: state.instances().map(|(id, _)| id.clone()).collect(),
        });
    }

    let mut points = ResetPoints::default();
    for candidate in candidates {
        let after = usize::try_from(candidate.point.sequence).unwrap_or(usize::MAX);
        let block = candidate.block.or_else(|| {
            journal.iter().skip(after).find_map(|later| {
                touched(later, &candidate.nodes, &candidate.instances).map(|node| {
                    ResetBlock::ChangedLater {
                        event_type: later.event().name(),
                        node,
                    }
                })
            })
        });
        match block {
            None => points.eligible.push(candidate.point),
            Some(block) => points.blocked.push((candidate.point, block)),
        }
    }
    Ok(points)
}

/// The effects a step that a reset runs again may repeat, from its declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EffectMarkers {
    pub compensate_with: bool,
    pub reconcile_with: bool,
    pub irreversible: bool,
    pub high_risk: bool,
}

/// A step the reset keeps as it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoredStep {
    pub node: NodeId,
    /// A decision of an approval, which the reset keeps instead of asking again.
    pub approval: bool,
}

/// A step the source ran after the point, which the reset run runs again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReexecutedStep {
    pub node: NodeId,
    /// It completed in the source, so running it again repeats an effect: the caller must acknowledge it
    /// (`acknowledgeReexecution`).
    pub completed_in_source: bool,
    pub effects: EffectMarkers,
}

/// What a reset to one point would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetPlan {
    pub point: ResetPoint,
    pub restored: Vec<RestoredStep>,
    pub reexecuted: Vec<ReexecutedStep>,
}

impl ResetPlan {
    /// Whether the caller has to acknowledge a step that completed in the source being run again.
    #[must_use]
    pub fn needs_acknowledgement(&self) -> bool {
        self.reexecuted.iter().any(|step| step.completed_in_source)
    }
}

/// Why a reset to a sequence is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResetRefusal {
    /// The sequence is not a candidate point of the journal.
    NotAPoint,
    /// It is a candidate and is blocked.
    Blocked(ResetBlock),
}

fn markers(def: &WorkflowDefinition, node: &NodeId) -> EffectMarkers {
    let Some(declared) = def.node(node) else {
        return EffectMarkers::default();
    };
    let effect = declared.effect();
    let (compensate_with, reconcile_with) = match declared.kind() {
        NodeKind::Activity {
            compensate_with,
            reconcile_with,
            ..
        } => (compensate_with.is_some(), reconcile_with.is_some()),
        _ => (false, false),
    };
    EffectMarkers {
        compensate_with,
        reconcile_with,
        irreversible: effect.irreversible,
        high_risk: effect.high_risk,
    }
}

/// What a reset to `sequence` would restore and run again.
///
/// The restored steps are those of the replayed state at the point; the steps run again are the declared
/// ones the source ran after it, each marked with the effects it declares and whether it completed.
///
/// # Errors
/// [`ResetRefusal`] unless `sequence` is an eligible point; a journal the reducer rejects is refused as not
/// having the point.
pub fn reset_plan(
    def: &WorkflowDefinition,
    run_id: RunId,
    journal: &[AddressedRunEvent],
    sequence: u64,
) -> Result<ResetPlan, ResetRefusal> {
    let points = reset_points(run_id, journal).map_err(|_| ResetRefusal::NotAPoint)?;
    if let Some((_, block)) = points
        .blocked
        .iter()
        .find(|(point, _)| point.sequence == sequence)
    {
        return Err(ResetRefusal::Blocked(block.clone()));
    }
    let point = points
        .eligible
        .into_iter()
        .find(|point| point.sequence == sequence)
        .ok_or(ResetRefusal::NotAPoint)?;
    let after = usize::try_from(sequence).unwrap_or(usize::MAX);
    let prefix = journal.get(..after).ok_or(ResetRefusal::NotAPoint)?;
    let state = RunState::replay_addressed(run_id, prefix).map_err(|_| ResetRefusal::NotAPoint)?;
    let restored: Vec<RestoredStep> = state
        .steps()
        .map(|(node, _)| RestoredStep {
            node: node.clone(),
            approval: matches!(
                def.node(node).map(crate::workflow::Node::kind),
                Some(NodeKind::Approval { .. })
            ),
        })
        .collect();
    let kept: BTreeSet<&NodeId> = restored.iter().map(|step| &step.node).collect();
    let mut seen: Vec<ReexecutedStep> = Vec::new();
    for later in journal
        .iter()
        .skip(after)
        .filter(|later| later.path().is_root())
    {
        let Some(node) = later.event().node() else {
            continue;
        };
        let node = forward_of(node);
        // only a node the author declared is a step the reset runs again; a compensation or a
        // reconciliation is the saga machinery's
        if kept.contains(&node)
            || !matches!(def.node(&node), Some(declared) if declared.origin().forward().is_none())
        {
            continue;
        }
        let completed = matches!(later.event(), RunEvent::StepCompleted { .. });
        match seen.iter_mut().find(|step| step.node == node) {
            Some(step) => step.completed_in_source |= completed,
            None => seen.push(ReexecutedStep {
                effects: markers(def, &node),
                node,
                completed_in_source: completed,
            }),
        }
    }
    Ok(ResetPlan {
        point,
        restored,
        reexecuted: seen,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use crate::execution::{FailureCode, PlannedEntry, ReviewOutcome};

    fn node(id: &str) -> NodeId {
        NodeId::parse_stored(id).expect("a node id")
    }

    fn prefix(ids: &[&str]) -> BTreeSet<NodeId> {
        ids.iter().map(|id| node(id)).collect()
    }

    fn root(event: RunEvent) -> AddressedRunEvent {
        AddressedRunEvent::root(event)
    }

    #[test]
    fn a_collection_that_did_not_succeed_is_not_a_quiet_state() {
        use crate::execution::{CollectionDrainReason, CollectionOutcome, InstancePath};
        use crate::workflow::Digest;
        let items = node("items");
        let body = ExecutionInstanceId::in_path(
            InstancePath::root()
                .child(items.clone(), 0)
                .expect("a path"),
            node("work"),
        );
        let code = FailureCode::worker_error();
        let events = [
            RunEvent::RunStarted {
                entrypoint: items.clone(),
            },
            RunEvent::CollectionEntered {
                node: items.clone(),
                item_count: 1,
                max_items: 1,
                max_concurrency: 1,
                snapshot_digest: Digest::new([0x51; 32]),
            },
            RunEvent::CollectionIterationAdmitted {
                node: items.clone(),
                index: 0,
                entry: body,
                item_digest: Digest::new([0; 32]),
            },
            RunEvent::CollectionDrainStarted {
                node: items.clone(),
                reason: CollectionDrainReason::Failed {
                    iteration: 0,
                    code: code.clone(),
                },
            },
            RunEvent::CollectionIterationDrained {
                node: items.clone(),
                index: 0,
            },
            RunEvent::CollectionFinished {
                node: items,
                outcome: CollectionOutcome::Failed(code),
                result_digest: None,
            },
        ];
        let state = RunState::replay(RunId::generate(), &events).expect("the journal replays");
        // nothing is open and the run is still running, but the collection ended in failure
        assert_eq!(state_block(&state, false), Some(ResetBlock::ActiveScope));
    }

    #[test]
    fn a_later_event_that_compensates_skips_resolves_or_reconciles_a_prefix_instance_is_found() {
        let kept = prefix(&["reserve"]);
        let none = BTreeSet::new();
        // a plan that undoes a node the prefix holds, as an entry and as an effect that cannot be undone
        let plan = |entry: &str, irreversible: &str| RunEvent::CompensationPlanned {
            node: node("s"),
            entries: vec![PlannedEntry {
                node: node(entry),
                after: Vec::new(),
            }],
            irreversible: vec![node(irreversible)],
        };
        assert_eq!(
            touched(&root(plan("reserve", "other")), &kept, &none),
            Some(node("reserve"))
        );
        assert_eq!(
            touched(&root(plan("other", "reserve")), &kept, &none),
            Some(node("reserve"))
        );
        assert_eq!(touched(&root(plan("other", "another")), &kept, &none), None);
        // a compensation or a reconciliation of a node is about the node it derives from
        for event in [
            RunEvent::StepSkipped {
                node: node("reserve~c"),
            },
            RunEvent::StepResolved {
                node: node("reserve~c"),
                outcome: ReviewOutcome::Succeeded,
            },
            RunEvent::StepNeedsReview {
                node: node("reserve"),
                code: FailureCode::parse("OUTCOME_UNKNOWN").expect("code"),
                fence: None,
            },
        ] {
            assert_eq!(touched(&root(event), &kept, &none), Some(node("reserve")));
        }
        // a node the prefix does not hold, and an event that is about none of this, are not
        assert_eq!(
            touched(
                &root(RunEvent::StepSkipped {
                    node: node("other~c")
                }),
                &kept,
                &none
            ),
            None
        );
        assert_eq!(touched(&root(RunEvent::RunPaused), &kept, &none), None);
    }
}
