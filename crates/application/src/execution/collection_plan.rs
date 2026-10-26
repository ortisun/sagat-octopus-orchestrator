//! Pure planning for bounded collection transitions.
//!
//! The engine deliberately does not call this module yet. Before it can do so, `RunTx` must persist
//! addressed events and outputs, scoped activities must support completion and routing, the worker
//! protocol must claim and report scoped identities, and collection checkpoint/finish storage must
//! commit projections, quota reservations and outbox rows with the journal.

#![allow(dead_code)]

use sagat_domain::execution::{
    AddressedRunEvent, CollectionPhase, EventAddressError, ExecutionError, ExecutionInstanceId,
    InstanceIdentityError, RunEvent, RunState,
};
use sagat_domain::workflow::{Digest, NodeKind, WorkflowDefinition};

/// Immutable membership metadata retained beside the captured collection values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CollectionSnapshot {
    digest: Digest,
    item_digests: Vec<Digest>,
}

impl CollectionSnapshot {
    #[must_use]
    pub(super) const fn new(digest: Digest, item_digests: Vec<Digest>) -> Self {
        Self {
            digest,
            item_digests,
        }
    }
}

/// Maximum new iterations one planner invocation may admit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AdmissionWindow(u16);

impl AdmissionWindow {
    pub(super) fn new(value: u16) -> Result<Self, CollectionPlanError> {
        if (1..=256).contains(&value) {
            Ok(Self(value))
        } else {
            Err(CollectionPlanError::InvalidAdmissionWindow)
        }
    }
}

/// When another planning transaction can make progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CollectionContinuation {
    None,
    ContinueAfterCommit,
    WaitForIteration,
    FinishCollection,
}

/// Ordered events that must be committed as one unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CollectionPlan {
    events: Vec<AddressedRunEvent>,
    continuation: CollectionContinuation,
}

/// Durable metadata produced when one body reaches its declared return node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CollectionReturn<'a> {
    index: u32,
    instance: &'a ExecutionInstanceId,
    result_digest: Digest,
}

impl<'a> CollectionReturn<'a> {
    #[must_use]
    pub(super) const fn new(
        index: u32,
        instance: &'a ExecutionInstanceId,
        result_digest: Digest,
    ) -> Self {
        Self {
            index,
            instance,
            result_digest,
        }
    }
}

impl CollectionPlan {
    #[must_use]
    pub(super) fn events(&self) -> &[AddressedRunEvent] {
        &self.events
    }

    #[must_use]
    pub(super) const fn continuation(&self) -> CollectionContinuation {
        self.continuation
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(super) enum CollectionPlanError {
    #[error("the collection definition does not match its logical instance")]
    InvalidDefinition,
    #[error("the captured collection contains {actual} items but allows at most {maximum}")]
    TooManyItems { actual: usize, maximum: u32 },
    #[error("the captured collection no longer matches durable progress")]
    SnapshotChanged,
    #[error("an admission window must be in 1..=256")]
    InvalidAdmissionWindow,
    #[error("the collection instance path cannot contain another iteration")]
    InvalidInstancePath,
    #[error("the return instance is not the declared body return")]
    InvalidReturn,
    #[error(transparent)]
    Reducer(#[from] ExecutionError),
    #[error(transparent)]
    Address(#[from] EventAddressError),
}

/// Plans collection entry and the first bounded group of logical iterations.
pub(super) fn plan_collection_entry(
    definition: &WorkflowDefinition,
    state: &RunState,
    collection: &ExecutionInstanceId,
    snapshot: &CollectionSnapshot,
    window: AdmissionWindow,
) -> Result<CollectionPlan, CollectionPlanError> {
    let (_, max_items, max_concurrency) = collection_definition(definition, collection)?;
    if snapshot.item_digests.len() > max_items as usize {
        return Err(CollectionPlanError::TooManyItems {
            actual: snapshot.item_digests.len(),
            maximum: max_items,
        });
    }
    let item_count = u32::try_from(snapshot.item_digests.len()).map_err(|_| {
        CollectionPlanError::TooManyItems {
            actual: snapshot.item_digests.len(),
            maximum: max_items,
        }
    })?;
    let entered = addressed(
        collection,
        RunEvent::CollectionEntered {
            node: collection.node().clone(),
            item_count,
            max_items,
            max_concurrency,
            snapshot_digest: snapshot.digest,
        },
    )?;
    let mut shadow = state.clone();
    shadow.apply_addressed(&entered)?;
    let mut continuation = plan_open_collection(definition, &shadow, collection, snapshot, window)?;
    let mut events = vec![entered];
    events.append(&mut continuation.events);
    validate_events(state, &events)?;
    Ok(CollectionPlan {
        events,
        continuation: continuation.continuation,
    })
}

/// Records one explicit body return, checkpoints its result and fills newly available capacity.
pub(super) fn plan_collection_return(
    definition: &WorkflowDefinition,
    state: &RunState,
    collection: &ExecutionInstanceId,
    returned: CollectionReturn<'_>,
    snapshot: &CollectionSnapshot,
    window: AdmissionWindow,
) -> Result<CollectionPlan, CollectionPlanError> {
    let returned_node = definition
        .node_for_instance(returned.instance)
        .ok_or(CollectionPlanError::InvalidReturn)?;
    if !matches!(returned_node.kind(), NodeKind::Return { .. }) {
        return Err(CollectionPlanError::InvalidReturn);
    }
    let mut events = vec![
        addressed(
            collection,
            RunEvent::CollectionIterationReturned {
                node: collection.node().clone(),
                index: returned.index,
                return_instance: returned.instance.clone(),
            },
        )?,
        addressed(
            collection,
            RunEvent::CollectionIterationCheckpointed {
                node: collection.node().clone(),
                index: returned.index,
                return_instance: returned.instance.clone(),
                result_digest: returned.result_digest,
            },
        )?,
    ];
    let mut shadow = state.clone();
    for event in &events {
        shadow.apply_addressed(event)?;
    }
    let mut continuation = plan_open_collection(definition, &shadow, collection, snapshot, window)?;
    events.append(&mut continuation.events);
    validate_events(state, &events)?;
    Ok(CollectionPlan {
        events,
        continuation: continuation.continuation,
    })
}

fn plan_open_collection(
    definition: &WorkflowDefinition,
    state: &RunState,
    collection: &ExecutionInstanceId,
    snapshot: &CollectionSnapshot,
    window: AdmissionWindow,
) -> Result<CollectionPlan, CollectionPlanError> {
    let (body_entry, _, _) = collection_definition(definition, collection)?;
    let scope = state
        .collection(collection)
        .ok_or(CollectionPlanError::InvalidDefinition)?;
    if scope.phase() != CollectionPhase::Open
        || scope.item_count() as usize != snapshot.item_digests.len()
        || scope.snapshot_digest() != snapshot.digest
    {
        return Err(CollectionPlanError::SnapshotChanged);
    }
    for index in 0..scope.next_admission() {
        if scope.item_digest(index) != snapshot.item_digests.get(index as usize).copied() {
            return Err(CollectionPlanError::SnapshotChanged);
        }
    }

    if scope.next_admission() == scope.item_count() && scope.active_count() == 0 {
        if scope.ordered_results().count() == scope.item_count() as usize {
            return Ok(CollectionPlan {
                events: Vec::new(),
                continuation: CollectionContinuation::FinishCollection,
            });
        }
        return Err(CollectionPlanError::SnapshotChanged);
    }

    let available = usize::from(scope.max_concurrency()).saturating_sub(scope.active_count());
    let remaining = (scope.item_count() - scope.next_admission()) as usize;
    let admitted = available.min(remaining).min(usize::from(window.0));
    let mut events = Vec::with_capacity(admitted);
    for offset in 0..admitted {
        let index = scope.next_admission() + offset as u32;
        let path = collection
            .path()
            .child(collection.node().clone(), index)
            .map_err(map_instance_error)?;
        let entry = ExecutionInstanceId::in_path(path, body_entry.clone());
        events.push(addressed(
            collection,
            RunEvent::CollectionIterationAdmitted {
                node: collection.node().clone(),
                index,
                entry,
                item_digest: snapshot.item_digests[index as usize],
            },
        )?);
    }
    validate_events(state, &events)?;

    let next_admission = scope.next_admission() + admitted as u32;
    let active = scope.active_count() + admitted;
    let continuation =
        if next_admission < scope.item_count() && active < usize::from(scope.max_concurrency()) {
            CollectionContinuation::ContinueAfterCommit
        } else {
            CollectionContinuation::WaitForIteration
        };
    Ok(CollectionPlan {
        events,
        continuation,
    })
}

fn collection_definition(
    definition: &WorkflowDefinition,
    collection: &ExecutionInstanceId,
) -> Result<(sagat_domain::workflow::NodeId, u32, u16), CollectionPlanError> {
    let node = definition
        .node_for_instance(collection)
        .ok_or(CollectionPlanError::InvalidDefinition)?;
    let NodeKind::ForEach {
        body,
        max_items,
        max_concurrency,
        ..
    } = node.kind()
    else {
        return Err(CollectionPlanError::InvalidDefinition);
    };
    let entry = definition
        .body(body)
        .ok_or(CollectionPlanError::InvalidDefinition)?
        .entrypoint()
        .clone();
    Ok((entry, *max_items, *max_concurrency))
}

fn addressed(
    collection: &ExecutionInstanceId,
    event: RunEvent,
) -> Result<AddressedRunEvent, CollectionPlanError> {
    AddressedRunEvent::in_path(collection.path().clone(), event).map_err(Into::into)
}

fn validate_events(
    state: &RunState,
    events: &[AddressedRunEvent],
) -> Result<(), CollectionPlanError> {
    let mut shadow = state.clone();
    for event in events {
        shadow.apply_addressed(event)?;
    }
    Ok(())
}

fn map_instance_error(_: InstanceIdentityError) -> CollectionPlanError {
    CollectionPlanError::InvalidInstancePath
}

#[cfg(test)]
mod tests {
    use sagat_domain::execution::{
        CollectionPhase, ExecutionInstanceId, Plan, RunEvent, RunId, RunState, StepStatus,
    };
    use sagat_domain::identity::Slug;
    use sagat_domain::workflow::{
        BodyDefinition, Digest, Expression, Node, NodeId, NodeKind, WorkflowDefinition,
        WorkflowLimits,
    };

    use super::{
        AdmissionWindow, CollectionContinuation, CollectionReturn, CollectionSnapshot,
        plan_collection_entry, plan_collection_return,
    };

    fn id(value: &str) -> NodeId {
        NodeId::parse(value).expect("node id")
    }

    fn expr(value: &str) -> Expression {
        Expression::parse(value).expect("expression")
    }

    fn definition(max_concurrency: u16) -> WorkflowDefinition {
        let collection = Node::new(
            id("items"),
            NodeKind::ForEach {
                items: expr("input.items"),
                body: id("line"),
                result: expr("bodyResult"),
                max_items: 10,
                max_concurrency,
            },
        )
        .with_next(id("done"));
        let done = Node::new(id("done"), NodeKind::End { output: None });
        let returned = Node::new(
            id("return"),
            NodeKind::Return {
                result: expr("item"),
            },
        );
        let body =
            BodyDefinition::new(id("line"), id("return"), vec![returned]).expect("closed body");
        WorkflowDefinition::new_structured(
            Slug::parse("collection-plan").expect("workflow name"),
            id("items"),
            vec![collection, done],
            vec![body],
            WorkflowLimits::default(),
        )
        .expect("structured definition")
    }

    fn started() -> RunState {
        let mut state = RunState::new(RunId::generate());
        state
            .apply(&RunEvent::RunStarted {
                entrypoint: id("items"),
            })
            .expect("run starts");
        state
    }

    fn snapshot(count: u8) -> CollectionSnapshot {
        CollectionSnapshot::new(
            Digest::new([0x51; 32]),
            (0..count).map(|index| Digest::new([index; 32])).collect(),
        )
    }

    #[test]
    fn entry_admits_only_the_window_and_keeps_more_work_explicit() {
        let definition = definition(3);
        let state = started();
        let collection = ExecutionInstanceId::root(id("items"));

        let plan = plan_collection_entry(
            &definition,
            &state,
            &collection,
            &snapshot(5),
            AdmissionWindow::new(2).expect("window"),
        )
        .expect("entry plan");

        assert_eq!(plan.events().len(), 3);
        assert!(matches!(
            plan.events()[0].event(),
            RunEvent::CollectionEntered { item_count: 5, .. }
        ));
        assert!(matches!(
            plan.events()[1].event(),
            RunEvent::CollectionIterationAdmitted { index: 0, .. }
        ));
        assert!(matches!(
            plan.events()[2].event(),
            RunEvent::CollectionIterationAdmitted { index: 1, .. }
        ));
        assert_eq!(
            plan.continuation(),
            CollectionContinuation::ContinueAfterCommit
        );

        let mut reduced = state;
        for event in plan.events() {
            reduced
                .apply_addressed(event)
                .expect("planned event applies");
        }
        let scope = reduced.collection(&collection).expect("collection state");
        assert_eq!(scope.next_admission(), 2);
        assert_eq!(scope.active_count(), 2);
    }

    #[test]
    fn return_checkpoint_and_next_admission_are_one_ordered_plan() {
        let definition = definition(1);
        let collection = ExecutionInstanceId::root(id("items"));
        let captured = snapshot(2);
        let mut state = started();
        let entered = plan_collection_entry(
            &definition,
            &state,
            &collection,
            &captured,
            AdmissionWindow::new(1).expect("window"),
        )
        .expect("entry plan");
        for event in entered.events() {
            state.apply_addressed(event).expect("entry applies");
        }
        let returned = state
            .collection(&collection)
            .and_then(|scope| scope.entry_instance(0))
            .cloned()
            .expect("return instance");

        let plan = plan_collection_return(
            &definition,
            &state,
            &collection,
            CollectionReturn::new(0, &returned, Digest::new([0x80; 32])),
            &captured,
            AdmissionWindow::new(1).expect("window"),
        )
        .expect("return plan");

        assert!(matches!(
            plan.events()[0].event(),
            RunEvent::CollectionIterationReturned { index: 0, .. }
        ));
        assert!(matches!(
            plan.events()[1].event(),
            RunEvent::CollectionIterationCheckpointed { index: 0, .. }
        ));
        assert!(matches!(
            plan.events()[2].event(),
            RunEvent::CollectionIterationAdmitted { index: 1, .. }
        ));
        assert_eq!(
            plan.continuation(),
            CollectionContinuation::WaitForIteration
        );

        for event in plan.events() {
            state.apply_addressed(event).expect("return plan applies");
        }
        let scope = state.collection(&collection).expect("collection state");
        assert_eq!(scope.active_count(), 1);
        assert_eq!(scope.ordered_results().count(), 1);
        assert_eq!(
            state.instance(&returned).map(|step| step.status()),
            Some(StepStatus::Succeeded)
        );
    }

    #[test]
    fn empty_collection_enters_and_finishes_without_body_work() {
        let definition = definition(3);
        let state = started();
        let collection = ExecutionInstanceId::root(id("items"));

        let plan = plan_collection_entry(
            &definition,
            &state,
            &collection,
            &snapshot(0),
            AdmissionWindow::new(3).expect("window"),
        )
        .expect("empty plan");

        assert_eq!(plan.events().len(), 1);
        assert_eq!(
            plan.continuation(),
            CollectionContinuation::FinishCollection
        );

        let mut reduced = state;
        for event in plan.events() {
            reduced
                .apply_addressed(event)
                .expect("planned event applies");
        }
        assert_eq!(
            reduced
                .collection(&collection)
                .expect("collection state")
                .phase(),
            CollectionPhase::Open
        );
    }

    #[test]
    fn changed_snapshot_is_rejected_before_another_admission() {
        let definition = definition(1);
        let collection = ExecutionInstanceId::root(id("items"));
        let captured = snapshot(2);
        let mut state = started();
        let entered = plan_collection_entry(
            &definition,
            &state,
            &collection,
            &captured,
            AdmissionWindow::new(1).expect("window"),
        )
        .expect("entry plan");
        for event in entered.events() {
            state.apply_addressed(event).expect("entry applies");
        }
        let returned = state
            .collection(&collection)
            .and_then(|scope| scope.entry_instance(0))
            .cloned()
            .expect("return instance");
        let changed = CollectionSnapshot::new(
            captured.digest,
            vec![Digest::new([0xff; 32]), Digest::new([1; 32])],
        );

        let result = plan_collection_return(
            &definition,
            &state,
            &collection,
            CollectionReturn::new(0, &returned, Digest::new([0x80; 32])),
            &changed,
            AdmissionWindow::new(1).expect("window"),
        );

        assert_eq!(result, Err(super::CollectionPlanError::SnapshotChanged));
    }

    #[test]
    fn collection_owner_waits_for_the_atomic_store_path() {
        let definition = definition(1);
        let state = started();

        let result =
            sagat_domain::execution::plan(&definition, &state, time::OffsetDateTime::UNIX_EPOCH);

        assert_eq!(result, Ok(Plan::Idle));
    }
}
