//! Sagas: long-running business transactions with compensations and reconciliation (feature 003).
//!
//! A saga is a *scope* of the workflow graph (a `saga` node and the region it owns, like a
//! `parallel` node and its branches). When something inside the scope fails for good, the engine
//! stops the scope's forward work, builds the **compensation plan** from the steps that completed,
//! and runs their compensations in reverse dependency order. A compensation is a new business action,
//! not a rollback (`docs/architecture/sagas.md`).
//!
//! The types here describe what an author declares and what a saga ends as. How a run moves through
//! them is in [`crate::execution`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use crate::workflow::{ActivityName, NodeId, NodeKind, RetryPolicy, Scope, WorkflowDefinition};

/// What a saga does when one of its compensations fails after its retries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OnCompensationFailure {
    /// The compensation waits for an operator (retry, skip with a justification, or resolve it);
    /// the run is in `MANUAL_REVIEW`. The default: a half-compensated payment is not something to
    /// decide automatically.
    #[default]
    ManualReview,
    /// The saga ends as `COMPENSATION_FAILED` and the run with it; nothing waits for a human.
    Fail,
}

impl OnCompensationFailure {
    /// The name used in the IR.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ManualReview => "manual_review",
            Self::Fail => "fail",
        }
    }
}

impl FromStr for OnCompensationFailure {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, ()> {
        match value {
            "manual_review" => Ok(Self::ManualReview),
            "fail" => Ok(Self::Fail),
            _ => Err(()),
        }
    }
}

/// What happens to a saga when its run is cancelled or runs past its deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SagaCancelPolicy {
    /// Stop the forward work that is not in flight, ask what is in flight to stop, and compensate
    /// what completed. The default (`docs/architecture/sagas.md`, rule 6).
    #[default]
    CompensateCompleted,
    /// Stop everything and compensate nothing.
    AbortNoCompensation,
    /// Let the attempts in flight finish (their effect may already have happened), then compensate
    /// everything that completed, those included.
    WaitInFlightThenCompensate,
}

impl SagaCancelPolicy {
    /// The name used in the IR.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CompensateCompleted => "compensate_completed",
            Self::AbortNoCompensation => "abort_no_compensation",
            Self::WaitInFlightThenCompensate => "wait_in_flight_then_compensate",
        }
    }

    /// The name shown by the API.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::CompensateCompleted => "COMPENSATE_COMPLETED",
            Self::AbortNoCompensation => "ABORT_NO_COMPENSATION",
            Self::WaitInFlightThenCompensate => "WAIT_IN_FLIGHT_THEN_COMPENSATE",
        }
    }

    /// Whether the scope's attempts in flight are asked to stop.
    #[must_use]
    pub const fn cancels_in_flight(self) -> bool {
        !matches!(self, Self::WaitInFlightThenCompensate)
    }

    /// Whether anything is compensated.
    #[must_use]
    pub const fn compensates(self) -> bool {
        !matches!(self, Self::AbortNoCompensation)
    }
}

impl FromStr for SagaCancelPolicy {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, ()> {
        match value {
            "compensate_completed" => Ok(Self::CompensateCompleted),
            "abort_no_compensation" => Ok(Self::AbortNoCompensation),
            "wait_in_flight_then_compensate" => Ok(Self::WaitInFlightThenCompensate),
            _ => Err(()),
        }
    }
}

/// What an author says about the effect of an activity, besides which activities compensate and
/// reconcile it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EffectPolicy {
    /// The retry budget of the compensation; [`RetryPolicy::COMPENSATION`] when absent.
    pub compensation_retry: Option<RetryPolicy>,
    /// Longest one attempt of the compensation may take.
    pub compensation_timeout: Option<Duration>,
    /// The effect cannot be undone once it has happened. A saga that has to compensate past it ends
    /// `PARTIAL`, never `COMPENSATED`.
    pub irreversible: bool,
    /// An attempt whose outcome is unknown (lease lost, timeout, `OUTCOME_UNKNOWN`) must never be
    /// retried or compensated blindly: it is reconciled when the activity says how, and otherwise it
    /// waits for an operator (`FR-003-007`).
    pub high_risk: bool,
}

impl EffectPolicy {
    /// No statement: ordinary effect.
    pub const NONE: Self = Self {
        compensation_retry: None,
        compensation_timeout: None,
        irreversible: false,
        high_risk: false,
    };
}

/// Where a node of a definition comes from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeOrigin {
    /// Written by the author.
    Declared,
    /// The compensation of the forward node it names. Derived from `compensateWith`; nothing routes
    /// to it, the saga machinery runs it.
    Compensation(NodeId),
    /// The reconciliation of the forward node it names. Derived from `reconcileWith`.
    Reconciliation(NodeId),
}

impl NodeOrigin {
    /// The forward node a derived node belongs to.
    #[must_use]
    pub const fn forward(&self) -> Option<&NodeId> {
        match self {
            Self::Declared => None,
            Self::Compensation(node) | Self::Reconciliation(node) => Some(node),
        }
    }

    /// The name stored and shown (`FORWARD`, `COMPENSATION`, `RECONCILIATION`).
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Declared => "FORWARD",
            Self::Compensation(_) => "COMPENSATION",
            Self::Reconciliation(_) => "RECONCILIATION",
        }
    }
}

/// How a saga ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SagaOutcome {
    /// Everything that completed was compensated.
    Compensated,
    /// Something was not compensated: an irreversible effect, or a compensation an operator skipped.
    Partial,
    /// A compensation failed after its retries and the saga does not wait for a human.
    CompensationFailed,
}

impl SagaOutcome {
    /// The name stored and shown.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Compensated => "COMPENSATED",
            Self::Partial => "PARTIAL",
            Self::CompensationFailed => "COMPENSATION_FAILED",
        }
    }
}

impl fmt::Display for SagaOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SagaOutcome {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, ()> {
        match value {
            "COMPENSATED" => Ok(Self::Compensated),
            "PARTIAL" => Ok(Self::Partial),
            "COMPENSATION_FAILED" => Ok(Self::CompensationFailed),
            _ => Err(()),
        }
    }
}

/// What a reconciliation found out about an effect whose outcome was unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReconcileVerdict {
    /// The effect happened: the step succeeded.
    Applied,
    /// The effect did not happen: it is safe to try again.
    NotApplied,
    /// The provider cannot say. Never assumed either way: an operator decides.
    Unknown,
}

impl ReconcileVerdict {
    /// The name the activity reports and the journal stores.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::NotApplied => "not_applied",
            Self::Unknown => "unknown",
        }
    }
}

impl FromStr for ReconcileVerdict {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, ()> {
        match value {
            "applied" => Ok(Self::Applied),
            "not_applied" => Ok(Self::NotApplied),
            "unknown" => Ok(Self::Unknown),
            _ => Err(()),
        }
    }
}

/// One compensation of the plan of a saga, as the definition declares it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanEntry {
    /// The forward node the compensation undoes.
    pub node: NodeId,
    /// The compensation node (`node~c`).
    pub compensation: NodeId,
    /// The activity that undoes the node (`compensateWith`).
    pub activity: ActivityName,
    /// The forward nodes whose compensations must have finished before this one starts: those that
    /// come after this node in the graph and have a compensation of their own.
    pub after: Vec<NodeId>,
}

/// The compensation plan of one saga of a definition: a reverse topological order of the forward
/// nodes that have a compensation, with the branches of a `parallel` node independent of each other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SagaPlan {
    pub saga: NodeId,
    pub on_compensation_failure: OnCompensationFailure,
    pub cancel_policy: SagaCancelPolicy,
    /// In an order a worker could follow one by one: a compensation never precedes one it waits for.
    pub entries: Vec<PlanEntry>,
    /// Forward nodes of the saga whose effect cannot be undone.
    pub irreversible: Vec<NodeId>,
}

/// Where control goes after `node`, for the purpose of ordering: where its own edges lead and,
/// when it ends a branch of a `parallel` node, where the join leads. The end of a branch has no
/// edge of its own, yet what comes after the join happened after it.
///
/// The continuation of a saga body is not followed: the plan of a saga orders only what is inside
/// it.
fn following<'a>(definition: &'a WorkflowDefinition, node: &'a NodeId) -> Vec<&'a NodeId> {
    let Some(found) = definition.node(node) else {
        return Vec::new();
    };
    let mut out = found.successors();
    if found.next().is_none() {
        for scope in definition.scope_chain(node) {
            match scope {
                Scope::Parallel(parallel) => {
                    if let Some(next) = definition.node(parallel).and_then(|p| p.next()) {
                        out.push(next);
                        break;
                    }
                    // The join leads nowhere by itself: it ends the branch of an outer one.
                }
                Scope::Saga(_) => break,
            }
        }
    }
    out
}

/// For every node of `entries`, the entries it has to wait for: the *nearest* entries downstream of
/// it, going through nodes that are not entries (a step that has nothing to undo, or that never ran).
///
/// Waiting for the nearest ones is the same as waiting for all of them, because each of those waits
/// for the ones behind it in turn, and it keeps the plan linear in the size of the saga instead of
/// quadratic: a chain of N compensations stores N links, not N(N-1)/2.
///
/// What lies between two entries is walked once however many entries lead into it (a tail shared by
/// thousands of branches is not walked thousands of times): the entries nearest to a node are worked
/// out once, from those of its successors.
#[must_use]
pub fn reverse_dependencies(
    definition: &WorkflowDefinition,
    entries: &BTreeSet<NodeId>,
) -> BTreeMap<NodeId, Vec<NodeId>> {
    // The nearest entries at or after a node: itself if it is one.
    let mut reach: BTreeMap<&NodeId, BTreeSet<&NodeId>> = BTreeMap::new();
    for entry in entries {
        for first in following(definition, entry) {
            // Post-order over the nodes below `first` that are not entries, without recursion (a
            // definition may be ten thousand nodes long).
            let mut stack: Vec<(&NodeId, bool)> = vec![(first, false)];
            while let Some((id, expanded)) = stack.pop() {
                if reach.contains_key(id) {
                    continue;
                }
                if entries.contains(id) {
                    reach.insert(id, BTreeSet::from([id]));
                    continue;
                }
                let next = following(definition, id);
                if expanded {
                    let mut nearest = BTreeSet::new();
                    for successor in next {
                        if let Some(found) = reach.get(successor) {
                            nearest.extend(found.iter().copied());
                        }
                    }
                    reach.insert(id, nearest);
                } else {
                    stack.push((id, true));
                    stack.extend(
                        next.into_iter()
                            .filter(|s| !reach.contains_key(s))
                            .map(|s| (s, false)),
                    );
                }
            }
        }
    }
    entries
        .iter()
        .map(|entry| {
            let mut after: BTreeSet<&NodeId> = BTreeSet::new();
            for first in following(definition, entry) {
                if let Some(found) = reach.get(first) {
                    after.extend(found.iter().copied());
                }
            }
            (entry.clone(), after.into_iter().cloned().collect())
        })
        .collect()
}

/// A topological order of `deps` in which a node comes after everything it waits for. Nodes that
/// wait for nothing are taken in the order of their ids, so the result is deterministic.
#[must_use]
pub fn reverse_order(deps: &BTreeMap<NodeId, Vec<NodeId>>) -> Vec<NodeId> {
    let mut waiting: BTreeMap<&NodeId, usize> = deps
        .iter()
        .map(|(node, after)| {
            let known = after.iter().filter(|a| deps.contains_key(*a)).count();
            (node, known)
        })
        .collect();
    let mut dependents: BTreeMap<&NodeId, Vec<&NodeId>> = BTreeMap::new();
    for (node, after) in deps {
        for earlier in after.iter().filter(|a| deps.contains_key(*a)) {
            dependents.entry(earlier).or_default().push(node);
        }
    }
    let mut ready: BTreeSet<&NodeId> = waiting
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(node, _)| *node)
        .collect();
    let mut ordered = Vec::with_capacity(deps.len());
    while let Some(node) = ready.pop_first() {
        ordered.push(node.clone());
        for dependent in dependents.get(node).into_iter().flatten() {
            if let Some(count) = waiting.get_mut(dependent) {
                *count -= 1;
                if *count == 0 {
                    ready.insert(dependent);
                }
            }
        }
    }
    if ordered.len() < deps.len() {
        // The definition is acyclic, so this cannot happen; an unordered tail is still better than
        // dropping compensations.
        let placed: BTreeSet<&NodeId> = ordered.iter().collect();
        let rest: Vec<NodeId> = deps
            .keys()
            .filter(|node| !placed.contains(node))
            .cloned()
            .collect();
        ordered.extend(rest);
    }
    ordered
}

/// The compensation plan of every saga of `definition`, in the order of the saga nodes' ids. It is a
/// pure function of the definition: publishing the same version always yields the same plan, which is
/// what lets it be stored once, immutable, with the version (`FR-003-002`).
#[must_use]
pub fn compensation_plans(definition: &WorkflowDefinition) -> Vec<SagaPlan> {
    let mut plans = Vec::new();
    for node in definition.nodes() {
        let NodeKind::Saga {
            on_compensation_failure,
            cancel_policy,
            ..
        } = node.kind()
        else {
            continue;
        };
        let region = definition
            .saga_region(node.id())
            .cloned()
            .unwrap_or_default();
        let compensable: BTreeSet<NodeId> = region
            .iter()
            .filter(|member| definition.node(&member.compensation()).is_some())
            .cloned()
            .collect();
        let deps = reverse_dependencies(definition, &compensable);
        // Waiting for the nodes downstream means those are compensated first: the order is the
        // reverse of the forward one, with independent branches in any order.
        let entries = reverse_order(&deps)
            .into_iter()
            .filter_map(|forward| {
                let compensation = forward.compensation();
                let NodeKind::Activity { activity, .. } = definition.node(&compensation)?.kind()
                else {
                    return None;
                };
                Some(PlanEntry {
                    activity: activity.clone(),
                    after: deps.get(&forward).cloned().unwrap_or_default(),
                    compensation,
                    node: forward,
                })
            })
            .collect();
        let irreversible = region
            .iter()
            .filter(|member| {
                definition
                    .node(member)
                    .is_some_and(|n| n.effect().irreversible)
            })
            .cloned()
            .collect();
        plans.push(SagaPlan {
            saga: node.id().clone(),
            on_compensation_failure: *on_compensation_failure,
            cancel_policy: *cancel_policy,
            entries,
            irreversible,
        });
    }
    plans
}
