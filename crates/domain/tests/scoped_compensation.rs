//! Compensation and reconciliation of the members of a collection or loop scope (FR-014-005,
//! FR-014-006, FR-003-002, ADR-0030 item 7).
//!
//! Every scenario drives a run through the addressed commands the application uses, checks the next
//! decision of the pure scope planner, and checks that the journal alone rebuilds the state.
#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use sagat_domain::execution::{
    AddressedPlan, AddressedRunEvent, CollectionDrainReason, ExecutionError, ExecutionInstanceId,
    Expiry, Failure, FailureCode, Fence, InstancePath, LoopContinuation, LoopDrainReason, LoopKind,
    ReviewOutcome, RunEvent, RunId, RunState, RunStatus, SagaPhase, StepStatus, StopReason,
    WaitingOn, complete_instance, complete_reconciliation_instance, expire_instance, fail_instance,
    lease_instance, plan_scope_compensation, resolve_review_instance, retry_review_instance,
    skip_review_instance,
};
use sagat_domain::identity::Slug;
use sagat_domain::saga::{EffectPolicy, ReconcileVerdict, SagaOutcome as Outcome};
use sagat_domain::workflow::{
    ActivityName, BodyDefinition, Digest, Expression, Node, NodeId, NodeKind, RetryPolicy,
    WorkflowDefinition, WorkflowLimits,
};
use time::OffsetDateTime;

const T0: OffsetDateTime = OffsetDateTime::UNIX_EPOCH;

fn id(value: &str) -> NodeId {
    NodeId::parse_stored(value).expect("valid stored node id")
}

fn expr(value: &str) -> Expression {
    Expression::parse(value).expect("expression")
}

/// An activity of a body; `undo`/`lookup` name the compensation and the reconciliation.
fn step(name: &str, undo: bool, lookup: bool) -> Node {
    Node::new(
        id(name),
        NodeKind::Activity {
            activity: ActivityName::parse(&format!("act.{name}")).expect("activity"),
            retry: RetryPolicy::DEFAULT,
            compensate_with: undo
                .then(|| ActivityName::parse(&format!("undo.{name}")).expect("activity")),
            reconcile_with: lookup
                .then(|| ActivityName::parse(&format!("lookup.{name}")).expect("activity")),
            idempotency_template: None,
            secrets: Vec::new(),
        },
    )
}

fn irreversible(name: &str) -> Node {
    step(name, false, false).with_effect(EffectPolicy {
        irreversible: true,
        ..EffectPolicy::NONE
    })
}

/// A chain of the given activities, in order, then the body's return.
fn chain(nodes: Vec<Node>) -> (NodeId, Vec<Node>) {
    let entry = nodes.first().expect("a body has an activity").id().clone();
    let mut linked = Vec::new();
    let count = nodes.len();
    for (position, node) in nodes.into_iter().enumerate() {
        let next = if position + 1 == count {
            id("returned")
        } else {
            // The next activity is read from the following element below.
            id("placeholder")
        };
        linked.push((node, next));
    }
    let names: Vec<NodeId> = linked.iter().map(|(node, _)| node.id().clone()).collect();
    let mut out = Vec::new();
    for (position, (node, next)) in linked.into_iter().enumerate() {
        let next = if position + 1 < names.len() {
            names[position + 1].clone()
        } else {
            next
        };
        out.push(node.with_next(next));
    }
    out.push(Node::new(
        id("returned"),
        NodeKind::Return {
            result: expr("steps.reserve.output"),
        },
    ));
    (entry, out)
}

fn collection_definition(nodes: Vec<Node>, max_concurrency: u16) -> WorkflowDefinition {
    let (entry, body_nodes) = chain(nodes);
    let body = BodyDefinition::new(id("line"), entry, body_nodes).expect("closed body");
    WorkflowDefinition::new_structured(
        Slug::parse("scoped-compensation").expect("workflow name"),
        id("lines"),
        vec![
            Node::new(
                id("lines"),
                NodeKind::ForEach {
                    items: expr("input.items"),
                    body: id("line"),
                    result: expr("bodyResult"),
                    max_items: 8,
                    max_concurrency,
                },
            )
            .with_next(id("done")),
            Node::new(id("done"), NodeKind::End { output: None }),
        ],
        vec![body],
        WorkflowLimits::default(),
    )
    .expect("collection workflow")
}

fn loop_definition(nodes: Vec<Node>, kind: LoopKind) -> WorkflowDefinition {
    let (entry, body_nodes) = chain(nodes);
    let body = BodyDefinition::new(id("pass"), entry, body_nodes).expect("closed body");
    let owner = match kind {
        LoopKind::While => NodeKind::While {
            condition: expr("carry < 9"),
            body: id("pass"),
            initial: expr("0"),
            carry: expr("bodyResult"),
            max_iterations: 8,
            delay: None,
        },
        LoopKind::RepeatUntil => NodeKind::RepeatUntil {
            condition: expr("carry >= 9"),
            body: id("pass"),
            initial: expr("0"),
            carry: expr("bodyResult"),
            max_iterations: 8,
            delay: None,
        },
    };
    WorkflowDefinition::new_structured(
        Slug::parse("scoped-loop").expect("workflow name"),
        id("poll"),
        vec![
            Node::new(id("poll"), owner)
                .with_timeout(Duration::from_secs(600))
                .with_next(id("done")),
            Node::new(id("done"), NodeKind::End { output: None }),
        ],
        vec![body],
        WorkflowLimits::default(),
    )
    .expect("loop workflow")
}

/// One run of a structured workflow, driven by addressed events and journaled in order.
struct Scope {
    def: WorkflowDefinition,
    state: RunState,
    journal: Vec<AddressedRunEvent>,
    owner: ExecutionInstanceId,
}

impl Scope {
    fn start(def: WorkflowDefinition, owner: &str) -> Self {
        let owner = ExecutionInstanceId::root(id(owner));
        let mut scope = Self {
            def,
            state: RunState::new(RunId::generate()),
            journal: Vec::new(),
            owner: owner.clone(),
        };
        scope.event(RunEvent::RunStarted {
            entrypoint: owner.node().clone(),
        });
        scope
    }

    fn event(&mut self, event: RunEvent) {
        let addressed = AddressedRunEvent::root(event);
        self.state
            .apply_addressed(&addressed)
            .expect("event applies");
        self.journal.push(addressed);
    }

    fn addressed(&mut self, path: &InstancePath, event: RunEvent) {
        let addressed = AddressedRunEvent::in_path(path.clone(), event).expect("address");
        self.state
            .apply_addressed(&addressed)
            .expect("event applies");
        self.journal.push(addressed);
    }

    fn path(&self, index: u32) -> InstancePath {
        InstancePath::root()
            .child(self.owner.node().clone(), index)
            .expect("bounded path")
    }

    fn at(&self, index: u32, node: &str) -> ExecutionInstanceId {
        ExecutionInstanceId::in_path(self.path(index), id(node))
    }

    fn absorb(&mut self, events: Vec<AddressedRunEvent>) {
        self.journal.extend(events);
    }

    // ---------------------------------------------------------------- collection forward work

    fn enter_collection(&mut self, items: u32, max_concurrency: u16) {
        self.event(RunEvent::CollectionEntered {
            node: self.owner.node().clone(),
            item_count: items,
            max_items: 8,
            max_concurrency,
            snapshot_digest: Digest::new([0x51; 32]),
        });
    }

    fn admit_collection(&mut self, index: u32, entry: &str) {
        let owner = self.owner.node().clone();
        let entry = self.at(index, entry);
        self.event(RunEvent::CollectionIterationAdmitted {
            node: owner,
            index,
            entry: entry.clone(),
            item_digest: Digest::new([index as u8; 32]),
        });
        self.addressed(
            entry.path(),
            RunEvent::StepReleased {
                node: entry.node().clone(),
            },
        );
    }

    fn checkpoint_collection(&mut self, index: u32) {
        let owner = self.owner.node().clone();
        let returned = self.at(index, "returned");
        self.event(RunEvent::CollectionIterationReturned {
            node: owner.clone(),
            index,
            return_instance: returned.clone(),
        });
        self.event(RunEvent::CollectionIterationCheckpointed {
            node: owner,
            index,
            return_instance: returned,
            result_digest: Digest::new([0x70 + index as u8; 32]),
        });
    }

    fn fail_collection(&mut self, index: u32, code: &str) {
        let code = FailureCode::parse(code).expect("code");
        self.event(RunEvent::RunStopRequested {
            reason: StopReason::Failed,
            code: Some(code.clone()),
        });
        self.event(RunEvent::CollectionDrainStarted {
            node: self.owner.node().clone(),
            reason: CollectionDrainReason::Failed {
                iteration: index,
                code,
            },
        });
    }

    fn cancel_collection(&mut self) {
        self.event(RunEvent::RunStopRequested {
            reason: StopReason::Cancelled,
            code: None,
        });
        self.event(RunEvent::CollectionDrainStarted {
            node: self.owner.node().clone(),
            reason: CollectionDrainReason::Cancelled,
        });
    }

    fn drain_collection(&mut self, index: u32) {
        self.event(RunEvent::CollectionIterationDrained {
            node: self.owner.node().clone(),
            index,
        });
    }

    // ---------------------------------------------------------------------- loop forward work

    fn enter_loop(&mut self, kind: LoopKind) {
        self.event(RunEvent::LoopEntered {
            node: self.owner.node().clone(),
            kind,
            max_iterations: 8,
            entered_at: T0 + Duration::from_secs(10),
            deadline: T0 + Duration::from_secs(600),
            initial_carry_digest: Digest::new([0x10; 32]),
            initial_condition: matches!(kind, LoopKind::While).then_some(true),
        });
    }

    fn admit_loop(&mut self, index: u32, entry: &str) {
        let owner = self.owner.node().clone();
        let entry = self.at(index, entry);
        self.event(RunEvent::LoopIterationAdmitted {
            node: owner,
            index,
            entry: entry.clone(),
        });
        self.addressed(
            entry.path(),
            RunEvent::StepReleased {
                node: entry.node().clone(),
            },
        );
    }

    fn checkpoint_loop(&mut self, index: u32) {
        let owner = self.owner.node().clone();
        let returned = self.at(index, "returned");
        self.event(RunEvent::LoopIterationReturned {
            node: owner.clone(),
            index,
            return_instance: returned.clone(),
        });
        self.event(RunEvent::LoopIterationCheckpointed {
            node: owner,
            index,
            return_instance: returned,
            result_digest: Digest::new([0x20 + index as u8; 32]),
            carry_digest: Digest::new([0x30 + index as u8; 32]),
            condition: true,
            checked_at: T0 + Duration::from_secs(20 + u64::from(index)),
            continuation: LoopContinuation::Continue,
        });
    }

    fn fail_loop(&mut self, index: u32, code: &str) {
        let code = FailureCode::parse(code).expect("code");
        self.event(RunEvent::RunStopRequested {
            reason: StopReason::Failed,
            code: Some(code.clone()),
        });
        self.event(RunEvent::LoopDrainStarted {
            node: self.owner.node().clone(),
            reason: LoopDrainReason::Failed {
                iteration: index,
                code,
            },
        });
    }

    fn drain_loop(&mut self, index: u32) {
        self.event(RunEvent::LoopIterationDrained {
            node: self.owner.node().clone(),
            index,
        });
    }

    // -------------------------------------------------------------------------- member work

    fn lease(&mut self, instance: &ExecutionInstanceId) -> Fence {
        let (fence, event) = lease_instance(&mut self.state, instance).expect("lease");
        self.journal.push(event);
        fence
    }

    fn complete(&mut self, instance: &ExecutionInstanceId) {
        let fence = self.lease(instance);
        let progress = complete_instance(&self.def, &mut self.state, instance, fence)
            .expect("completion applies");
        self.absorb(progress.events);
    }

    fn fail(&mut self, instance: &ExecutionInstanceId, code: &str, retryable: bool) {
        let fence = self.lease(instance);
        self.fail_with(instance, fence, code, retryable)
            .expect("failure applies");
    }

    fn fail_with(
        &mut self,
        instance: &ExecutionInstanceId,
        fence: Fence,
        code: &str,
        retryable: bool,
    ) -> Result<(), ExecutionError> {
        let progress = fail_instance(
            &self.def,
            &mut self.state,
            instance,
            Failure {
                fence,
                code: FailureCode::parse(code).expect("code"),
                retryable,
                jitter_sample: 0.0,
            },
            T0,
        )?;
        self.absorb(progress.events);
        Ok(())
    }

    fn expire(&mut self, instance: &ExecutionInstanceId, fence: Fence, cause: Expiry) {
        let progress = expire_instance(&self.def, &mut self.state, instance, fence, cause, 0.0, T0)
            .expect("expiry applies");
        self.absorb(progress.events);
    }

    /// Plans and applies until the scope planner is idle; returns the events it chose.
    fn advance(&mut self) -> Vec<AddressedRunEvent> {
        let mut chosen = Vec::new();
        for _ in 0..256 {
            match plan_scope_compensation(&self.def, &self.state).expect("scope plan") {
                AddressedPlan::Emit(event) => {
                    self.state
                        .apply_addressed(&event)
                        .expect("planned event applies");
                    self.journal.push(event.clone());
                    chosen.push(event);
                }
                AddressedPlan::Idle => return chosen,
            }
        }
        panic!("the scope planner did not settle");
    }

    fn ready_compensation(&self) -> Option<ExecutionInstanceId> {
        self.state
            .instances()
            .find(|(i, s)| i.node().is_compensation() && s.status() == StepStatus::Ready)
            .map(|(i, _)| i.clone())
    }

    fn step_status(&self, instance: &ExecutionInstanceId) -> StepStatus {
        self.state.instance(instance).expect("step exists").status()
    }

    fn assert_replays(&self) {
        let replayed = RunState::replay_addressed(self.state.run_id(), &self.journal)
            .expect("the journal replays");
        assert_eq!(
            replayed, self.state,
            "replaying the journal must rebuild the state"
        );
    }
}

fn released(events: &[AddressedRunEvent]) -> Vec<(String, u32)> {
    events
        .iter()
        .filter_map(|addressed| match addressed.event() {
            RunEvent::StepReleased { node } => Some((
                node.as_str().to_owned(),
                addressed
                    .path()
                    .segments()
                    .last()
                    .map_or(0, |segment| segment.iteration_index()),
            )),
            _ => None,
        })
        .collect()
}

/// `reserve -> charge -> ship`, all compensable.
fn three_effects() -> Vec<Node> {
    vec![
        step("reserve", true, false),
        step("charge", true, false),
        step("ship", true, false),
    ]
}

/// A collection of two items: the first completed all three effects and was checkpointed, the second
/// completed `reserve` and `charge` and then `ship` failed for good, which drained the scope.
fn failed_after_a_completed_iteration() -> Scope {
    let mut scope = Scope::start(collection_definition(three_effects(), 2), "lines");
    scope.enter_collection(2, 2);
    scope.admit_collection(0, "reserve");
    scope.admit_collection(1, "reserve");
    for node in ["reserve", "charge", "ship"] {
        let instance = scope.at(0, node);
        scope.complete(&instance);
    }
    scope.checkpoint_collection(0);
    for node in ["reserve", "charge"] {
        let instance = scope.at(1, node);
        scope.complete(&instance);
    }
    let failing = scope.at(1, "ship");
    scope.fail(&failing, "PAYMENT_DECLINED", false);
    scope.fail_collection(1, "PAYMENT_DECLINED");
    scope.drain_collection(1);
    scope
}

// ------------------------------------------------------------------------------ order

#[test]
fn compensation_in_one_iteration_runs_in_reverse_topological_order() {
    let mut scope = failed_after_a_completed_iteration();

    let chosen = scope.advance();
    let planned = chosen
        .iter()
        .find_map(|addressed| match addressed.event() {
            RunEvent::ScopeCompensationPlanned { entries, .. } => Some(entries.clone()),
            _ => None,
        })
        .expect("the scope plans its compensation once the drain has settled");

    let after = |index: u32, node: &str| -> Vec<ExecutionInstanceId> {
        planned
            .iter()
            .find(|entry| entry.instance == scope.at(index, node))
            .unwrap_or_else(|| panic!("{node} of iteration {index} is planned"))
            .after
            .clone()
    };
    // The later effect is undone first; each waits for the nearest one after it.
    assert_eq!(after(0, "ship"), Vec::<ExecutionInstanceId>::new());
    assert_eq!(after(0, "charge"), vec![scope.at(0, "ship")]);
    assert_eq!(after(0, "reserve"), vec![scope.at(0, "charge")]);
    // The failed `ship` of iteration 1 never happened; its predecessors are undone in reverse.
    assert!(!planned.iter().any(|e| e.instance == scope.at(1, "ship")));
    assert_eq!(after(1, "charge"), Vec::<ExecutionInstanceId>::new());
    assert_eq!(after(1, "reserve"), vec![scope.at(1, "charge")]);
    assert_eq!(planned.len(), 5);

    // Only what has nothing to wait for is released, and then each step as its dependency ends.
    assert_eq!(
        released(&chosen),
        vec![("ship~c".to_owned(), 0), ("charge~c".to_owned(), 1)]
    );
    let mut order = vec![("ship~c", 0), ("charge~c", 1)];
    loop {
        let waiting = scope
            .state
            .instances()
            .filter(|(instance, step)| {
                instance.node().is_compensation() && step.status() == StepStatus::Ready
            })
            .map(|(instance, _)| instance.clone())
            .next();
        let Some(instance) = waiting else { break };
        scope.complete(&instance);
        for event in scope.advance() {
            if let RunEvent::StepReleased { node } = event.event() {
                let index = event
                    .path()
                    .segments()
                    .last()
                    .map_or(0, |segment| segment.iteration_index());
                order.push((
                    Box::leak(node.as_str().to_owned().into_boxed_str()) as &str,
                    index,
                ));
            }
        }
    }
    let per_iteration = |index: u32| -> Vec<&str> {
        order
            .iter()
            .filter(|(_, i)| *i == index)
            .map(|(node, _)| *node)
            .collect()
    };
    assert_eq!(per_iteration(0), ["ship~c", "charge~c", "reserve~c"]);
    assert_eq!(per_iteration(1), ["charge~c", "reserve~c"]);

    let saga = scope.state.scope_saga(&scope.owner).expect("scope saga");
    assert_eq!(saga.phase(), SagaPhase::Done);
    assert_eq!(saga.outcome(), Some(Outcome::Compensated));
    scope.assert_replays();
}

#[test]
fn iterations_of_a_collection_are_independent_branches() {
    let mut scope = failed_after_a_completed_iteration();
    let chosen = scope.advance();
    let planned = chosen
        .iter()
        .find_map(|addressed| match addressed.event() {
            RunEvent::ScopeCompensationPlanned { entries, .. } => Some(entries.clone()),
            _ => None,
        })
        .expect("planned");
    for entry in &planned {
        for earlier in &entry.after {
            assert_eq!(
                earlier.path(),
                entry.instance.path(),
                "no compensation waits for one of another iteration"
            );
        }
    }
    // Both iterations have something ready at once: the order between them is not a thing.
    let ready: Vec<_> = released(&chosen)
        .into_iter()
        .map(|(_, index)| index)
        .collect();
    assert_eq!(ready, vec![0, 1]);
}

#[test]
fn compensation_fan_out_is_bounded_by_the_scope_concurrency() {
    let mut scope = Scope::start(
        collection_definition(vec![step("reserve", true, false)], 1),
        "lines",
    );
    scope.enter_collection(3, 1);
    for index in 0..3 {
        scope.admit_collection(index, "reserve");
        let reserve = scope.at(index, "reserve");
        scope.complete(&reserve);
        if index < 2 {
            scope.checkpoint_collection(index);
        }
    }
    scope.fail_collection(2, "BOOM");
    scope.drain_collection(2);
    let chosen = scope.advance();
    // Three iterations, a scope concurrency of one: one compensation in flight at a time.
    assert_eq!(released(&chosen).len(), 1);
    let first = scope
        .state
        .instances()
        .find(|(i, s)| i.node().is_compensation() && s.status() == StepStatus::Ready)
        .map(|(i, _)| i.clone())
        .expect("one ready compensation");
    scope.complete(&first);
    assert_eq!(
        released(&scope.advance()).len(),
        1,
        "the next one is released"
    );
    scope.assert_replays();
}

#[test]
fn completed_iterations_of_a_loop_compensate_in_reverse_iteration_order() {
    let mut scope = Scope::start(
        loop_definition(
            vec![step("reserve", true, false), step("charge", true, false)],
            LoopKind::While,
        ),
        "poll",
    );
    scope.enter_loop(LoopKind::While);
    for index in 0..3 {
        scope.admit_loop(index, "reserve");
        for node in ["reserve", "charge"] {
            let instance = scope.at(index, node);
            scope.complete(&instance);
        }
        scope.checkpoint_loop(index);
    }
    scope.admit_loop(3, "reserve");
    let reserve = scope.at(3, "reserve");
    scope.complete(&reserve);
    let failing = scope.at(3, "charge");
    scope.fail(&failing, "PAYMENT_DECLINED", false);
    scope.fail_loop(3, "PAYMENT_DECLINED");
    scope.drain_loop(3);

    let chosen = scope.advance();
    // Iteration 3 holds the last effects: it is undone first, charge before reserve; iteration 2
    // cannot start until iteration 3 has finished, and so on.
    assert_eq!(released(&chosen), vec![("reserve~c".to_owned(), 3)]);
    let mut released_in_order = vec![(3_u32, "reserve~c".to_owned())];
    loop {
        let ready = scope
            .state
            .instances()
            .find(|(i, s)| i.node().is_compensation() && s.status() == StepStatus::Ready)
            .map(|(i, _)| i.clone());
        let Some(instance) = ready else { break };
        // Never a compensation of iteration k ready while a later iteration still has one open.
        let index = instance
            .path()
            .segments()
            .last()
            .map(|s| s.iteration_index())
            .expect("scoped");
        assert!(
            !scope.state.instances().any(|(other, step)| {
                other.node().is_compensation()
                    && !step.status().is_terminal()
                    && other
                        .path()
                        .segments()
                        .last()
                        .is_some_and(|s| s.iteration_index() > index)
                    && other != &instance
            }),
            "iteration {index} must wait for every later iteration"
        );
        scope.complete(&instance);
        for event in scope.advance() {
            if let RunEvent::StepReleased { node } = event.event() {
                released_in_order.push((
                    event
                        .path()
                        .segments()
                        .last()
                        .map_or(0, |s| s.iteration_index()),
                    node.as_str().to_owned(),
                ));
            }
        }
    }
    assert_eq!(
        released_in_order,
        vec![
            (3, "reserve~c".to_owned()),
            (2, "charge~c".to_owned()),
            (2, "reserve~c".to_owned()),
            (1, "charge~c".to_owned()),
            (1, "reserve~c".to_owned()),
            (0, "charge~c".to_owned()),
            (0, "reserve~c".to_owned()),
        ]
    );
    assert_eq!(
        scope
            .state
            .scope_saga(&scope.owner)
            .and_then(sagat_domain::execution::ScopeSagaState::outcome),
        Some(Outcome::Compensated)
    );
    scope.assert_replays();
}

// --------------------------------------------------------------------------- unknown effects

fn unknown_effect_scope() -> (Scope, ExecutionInstanceId, Fence) {
    let mut scope = Scope::start(
        collection_definition(vec![step("reserve", true, true)], 1),
        "lines",
    );
    scope.enter_collection(2, 1);
    scope.admit_collection(0, "reserve");
    let reserve = scope.at(0, "reserve");
    let fence = scope.lease(&reserve);
    (scope, reserve, fence)
}

#[test]
fn an_attempt_with_an_unknown_outcome_is_looked_up_before_anything_is_repeated() {
    let (mut scope, reserve, fence) = unknown_effect_scope();
    scope.expire(&reserve, fence, Expiry::LeaseExpired);

    let lookup = scope.at(0, "reserve~r");
    assert_eq!(scope.step_status(&reserve), StepStatus::Waiting);
    assert!(matches!(
        scope.state.instance(&reserve).and_then(|s| s.waiting_on()),
        Some(WaitingOn::Reconciliation)
    ));
    assert_eq!(
        scope.step_status(&lookup),
        StepStatus::Ready,
        "the lookup runs"
    );
    scope.assert_replays();
}

#[test]
fn an_iteration_whose_member_is_being_looked_up_cannot_be_drained() {
    let (mut scope, reserve, fence) = unknown_effect_scope();
    scope.expire(&reserve, fence, Expiry::LeaseExpired);
    scope.cancel_collection();
    let owner = scope.owner.node().clone();
    let drained = scope
        .state
        .clone()
        .apply(&RunEvent::CollectionIterationDrained {
            node: owner,
            index: 0,
        });
    assert!(
        drained.is_err(),
        "the effect may have happened: the iteration is not settled until the lookup answers"
    );
}

#[test]
fn a_lookup_that_found_the_effect_makes_the_step_compensable() {
    let (mut scope, reserve, fence) = unknown_effect_scope();
    scope.expire(&reserve, fence, Expiry::LeaseExpired);
    scope.cancel_collection();
    let lookup = scope.at(0, "reserve~r");
    let lookup_fence = scope.lease(&lookup);
    let progress = complete_reconciliation_instance(
        &scope.def,
        &mut scope.state,
        &lookup,
        lookup_fence,
        ReconcileVerdict::Applied,
        0.0,
        T0,
    )
    .expect("verdict applies");
    scope.absorb(progress.events);
    assert_eq!(scope.step_status(&reserve), StepStatus::Succeeded);
    scope.drain_collection(0);
    let chosen = scope.advance();
    assert_eq!(released(&chosen), vec![("reserve~c".to_owned(), 0)]);
    scope.assert_replays();
}

#[test]
fn a_lookup_that_found_nothing_does_not_retry_a_stopping_scope_and_compensates_nothing() {
    let (mut scope, reserve, fence) = unknown_effect_scope();
    scope.expire(&reserve, fence, Expiry::LeaseExpired);
    scope.cancel_collection();
    let lookup = scope.at(0, "reserve~r");
    let lookup_fence = scope.lease(&lookup);
    let progress = complete_reconciliation_instance(
        &scope.def,
        &mut scope.state,
        &lookup,
        lookup_fence,
        ReconcileVerdict::NotApplied,
        0.0,
        T0,
    )
    .expect("verdict applies");
    scope.absorb(progress.events);
    let step = scope.state.instance(&reserve).expect("step");
    assert_eq!(step.status(), StepStatus::Failed);
    assert_eq!(
        step.failure(),
        Some(&FailureCode::reconciled_not_applied()),
        "it did not happen: it is not an unknown outcome any more"
    );
    scope.drain_collection(0);
    let chosen = scope.advance();
    assert!(released(&chosen).is_empty(), "there is nothing to undo");
    assert_eq!(
        scope
            .state
            .scope_saga(&scope.owner)
            .and_then(sagat_domain::execution::ScopeSagaState::outcome),
        Some(Outcome::Compensated)
    );
}

#[test]
fn a_lookup_that_cannot_say_waits_for_a_person() {
    let (mut scope, reserve, fence) = unknown_effect_scope();
    scope.expire(&reserve, fence, Expiry::LeaseExpired);
    let lookup = scope.at(0, "reserve~r");
    let lookup_fence = scope.lease(&lookup);
    let progress = complete_reconciliation_instance(
        &scope.def,
        &mut scope.state,
        &lookup,
        lookup_fence,
        ReconcileVerdict::Unknown,
        0.0,
        T0,
    )
    .expect("verdict applies");
    scope.absorb(progress.events);
    assert!(matches!(
        scope.state.instance(&reserve).and_then(|s| s.waiting_on()),
        Some(WaitingOn::Operator { .. })
    ));
    assert_eq!(scope.state.status(), RunStatus::ManualReview);
    scope.assert_replays();
}

#[test]
fn an_unknown_outcome_is_compensated_defensively_when_the_scope_stops() {
    // No lookup: the attempt was lost while the run asked it to stop; the step keeps saying so.
    let mut scope = Scope::start(
        collection_definition(vec![step("reserve", true, false)], 1),
        "lines",
    );
    scope.enter_collection(1, 1);
    scope.admit_collection(0, "reserve");
    let reserve = scope.at(0, "reserve");
    let fence = scope.lease(&reserve);
    scope.cancel_collection();
    scope.addressed(
        reserve.path(),
        RunEvent::StepCancelRequested {
            node: reserve.node().clone(),
        },
    );
    scope.expire(&reserve, fence, Expiry::LeaseExpired);
    assert_eq!(scope.step_status(&reserve), StepStatus::Cancelled);
    scope.drain_collection(0);
    let chosen = scope.advance();
    assert_eq!(
        released(&chosen),
        vec![("reserve~c".to_owned(), 0)],
        "an effect that may have happened is undone, and the compensation tolerates that it did not"
    );
}

// ------------------------------------------------------------------------- irreversible

#[test]
fn an_irreversible_effect_ends_the_scope_partial_after_the_rest_is_undone() {
    let mut scope = Scope::start(
        collection_definition(
            vec![step("reserve", true, false), irreversible("notify")],
            1,
        ),
        "lines",
    );
    scope.enter_collection(2, 1);
    scope.admit_collection(0, "reserve");
    for node in ["reserve", "notify"] {
        let instance = scope.at(0, node);
        scope.complete(&instance);
    }
    scope.checkpoint_collection(0);
    scope.admit_collection(1, "reserve");
    let failing = scope.at(1, "reserve");
    scope.fail(&failing, "PAYMENT_DECLINED", false);
    scope.fail_collection(1, "PAYMENT_DECLINED");
    scope.drain_collection(1);

    let chosen = scope.advance();
    let irreversible = chosen
        .iter()
        .find_map(|addressed| match addressed.event() {
            RunEvent::ScopeCompensationPlanned { irreversible, .. } => Some(irreversible.clone()),
            _ => None,
        })
        .expect("planned");
    assert_eq!(irreversible, vec![scope.at(0, "notify")]);
    let undo = scope.at(0, "reserve~c");
    scope.complete(&undo);
    scope.advance();
    assert_eq!(
        scope
            .state
            .scope_saga(&scope.owner)
            .and_then(sagat_domain::execution::ScopeSagaState::outcome),
        Some(Outcome::Partial),
        "what could not be undone is never reported as compensated"
    );
    scope.assert_replays();
}

// --------------------------------------------------------------- a compensation that fails

fn one_compensation_that_keeps_failing() -> (Scope, ExecutionInstanceId) {
    let mut scope = Scope::start(
        collection_definition(vec![step("reserve", true, false)], 1),
        "lines",
    );
    scope.enter_collection(2, 1);
    scope.admit_collection(0, "reserve");
    let reserve = scope.at(0, "reserve");
    scope.complete(&reserve);
    scope.checkpoint_collection(0);
    scope.admit_collection(1, "reserve");
    let failing = scope.at(1, "reserve");
    scope.fail(&failing, "PAYMENT_DECLINED", false);
    scope.fail_collection(1, "PAYMENT_DECLINED");
    scope.drain_collection(1);
    scope.advance();
    let undo = scope.at(0, "reserve~c");
    for attempt in 1..=5 {
        let fence = scope.lease(&undo);
        scope
            .fail_with(&undo, fence, "PROVIDER_DOWN", true)
            .expect("failure applies");
        if attempt < 5 {
            assert_eq!(scope.step_status(&undo), StepStatus::RetryWait);
            let progress = sagat_domain::execution::retry_due_instance(
                &scope.def,
                &mut scope.state,
                &undo,
                T0 + Duration::from_secs(3600),
            )
            .expect("retry due");
            scope.absorb(progress.events);
        }
    }
    (scope, undo)
}

#[test]
fn a_compensation_that_keeps_failing_waits_for_a_person_and_is_never_assumed_done() {
    let (mut scope, undo) = one_compensation_that_keeps_failing();
    assert_eq!(scope.step_status(&undo), StepStatus::Waiting);
    assert!(matches!(
        scope.state.instance(&undo).and_then(|s| s.waiting_on()),
        Some(WaitingOn::Operator { .. })
    ));
    assert_eq!(scope.state.status(), RunStatus::ManualReview);
    assert!(
        scope.advance().is_empty(),
        "nothing is decided for the person"
    );
    assert_eq!(
        scope
            .state
            .scope_saga(&scope.owner)
            .map(sagat_domain::execution::ScopeSagaState::phase),
        Some(SagaPhase::Compensating)
    );
    scope.assert_replays();
}

#[test]
fn an_operator_retry_runs_the_same_compensation_again() {
    let (mut scope, undo) = one_compensation_that_keeps_failing();
    let progress = retry_review_instance(&scope.def, &mut scope.state, &undo, T0).expect("retry");
    scope.absorb(progress.events);
    assert_eq!(scope.step_status(&undo), StepStatus::Ready);
    scope.complete(&undo);
    scope.advance();
    assert_eq!(
        scope
            .state
            .scope_saga(&scope.owner)
            .and_then(sagat_domain::execution::ScopeSagaState::outcome),
        Some(Outcome::Compensated)
    );
    scope.assert_replays();
}

#[test]
fn an_operator_skip_leaves_the_scope_partial() {
    let (mut scope, undo) = one_compensation_that_keeps_failing();
    let progress = skip_review_instance(&scope.def, &mut scope.state, &undo, T0).expect("skip");
    scope.absorb(progress.events);
    scope.advance();
    assert_eq!(
        scope
            .state
            .scope_saga(&scope.owner)
            .and_then(sagat_domain::execution::ScopeSagaState::outcome),
        Some(Outcome::Partial)
    );
}

#[test]
fn an_operator_declaring_the_compensation_failed_ends_the_scope_compensation_failed() {
    let (mut scope, undo) = one_compensation_that_keeps_failing();
    let progress = resolve_review_instance(
        &scope.def,
        &mut scope.state,
        &undo,
        ReviewOutcome::Failed(FailureCode::compensation_failed()),
        T0,
    )
    .expect("resolve");
    scope.absorb(progress.events);
    scope.advance();
    assert_eq!(
        scope
            .state
            .scope_saga(&scope.owner)
            .and_then(sagat_domain::execution::ScopeSagaState::outcome),
        Some(Outcome::CompensationFailed)
    );
}

// ------------------------------------------------------------ outer cancellation and deadline

#[test]
fn a_cancellation_that_lands_during_compensation_changes_nothing() {
    let mut scope = failed_after_a_completed_iteration();
    scope.advance();
    // The run is already stopping for the failure: the second stop is refused and audited by the
    // caller, and the compensation goes on.
    for reason in [StopReason::Cancelled, StopReason::TimedOut] {
        let refused = scope
            .state
            .clone()
            .apply(&RunEvent::RunStopRequested { reason, code: None });
        assert_eq!(refused, Err(ExecutionError::AlreadyStopping));
    }
    assert_eq!(scope.state.status(), RunStatus::Compensating);
    let ready = scope
        .state
        .instances()
        .filter(|(i, s)| i.node().is_compensation() && s.status() == StepStatus::Ready)
        .count();
    assert_eq!(
        ready, 2,
        "the compensations are still leasable while the run is stopping"
    );
}

#[test]
fn a_stop_from_outside_never_gives_up_on_a_compensation_that_waits_for_a_person() {
    let mut scope = Scope::start(
        collection_definition(vec![step("reserve", true, false)], 1),
        "lines",
    );
    scope.enter_collection(1, 1);
    scope.admit_collection(0, "reserve");
    let reserve = scope.at(0, "reserve");
    scope.complete(&reserve);
    scope.checkpoint_collection(0);
    // The run is cancelled after the iteration completed; the scope drains and compensates.
    scope.cancel_collection();
    scope.advance();
    let undo = scope.at(0, "reserve~c");
    for _ in 0..5 {
        let fence = scope.lease(&undo);
        scope
            .fail_with(&undo, fence, "PROVIDER_DOWN", true)
            .expect("failure applies");
        if scope.step_status(&undo) == StepStatus::RetryWait {
            let progress = sagat_domain::execution::retry_due_instance(
                &scope.def,
                &mut scope.state,
                &undo,
                T0 + Duration::from_secs(3600),
            )
            .expect("retry due");
            scope.absorb(progress.events);
        }
    }
    assert_eq!(scope.state.status(), RunStatus::ManualReview);
    // Neither another planner pass nor a deadline gives up on the person.
    assert!(scope.advance().is_empty());
    assert_eq!(
        scope.state.clone().apply(&RunEvent::RunStopRequested {
            reason: StopReason::TimedOut,
            code: None,
        }),
        Err(ExecutionError::AlreadyStopping)
    );
    assert_eq!(scope.step_status(&undo), StepStatus::Waiting);
    // The person's decision is what ends the run, and a cancelled run whose compensation was
    // skipped is `PARTIAL`, not `CANCELLED`.
    let progress = skip_review_instance(&scope.def, &mut scope.state, &undo, T0).expect("skip");
    scope.absorb(progress.events);
    scope.advance();
    assert_eq!(
        scope.state.final_status_for(StopReason::Cancelled),
        RunStatus::Partial
    );
}

#[test]
fn a_failure_whose_completed_iterations_were_all_undone_ends_compensated() {
    let mut scope = failed_after_a_completed_iteration();
    scope.advance();
    while let Some(instance) = scope.ready_compensation() {
        scope.complete(&instance);
        scope.advance();
    }
    assert_eq!(
        scope.state.final_status_for(StopReason::Failed),
        RunStatus::Compensated
    );
}

#[test]
fn a_cancellation_whose_completed_iterations_were_all_undone_ends_cancelled() {
    let mut scope = Scope::start(
        collection_definition(vec![step("reserve", true, false)], 1),
        "lines",
    );
    scope.enter_collection(1, 1);
    scope.admit_collection(0, "reserve");
    let reserve = scope.at(0, "reserve");
    scope.complete(&reserve);
    scope.checkpoint_collection(0);
    scope.cancel_collection();
    scope.advance();
    let undo = scope.at(0, "reserve~c");
    scope.complete(&undo);
    scope.advance();
    assert_eq!(
        scope.state.final_status_for(StopReason::Cancelled),
        RunStatus::Cancelled
    );
}

// -------------------------------------------------------------------- scopes with no compensation

#[test]
fn a_scope_that_declares_nothing_to_compensate_plans_nothing() {
    let mut scope = Scope::start(
        collection_definition(vec![step("reserve", false, false)], 1),
        "lines",
    );
    scope.enter_collection(1, 1);
    scope.admit_collection(0, "reserve");
    let failing = scope.at(0, "reserve");
    scope.fail(&failing, "BOOM", false);
    scope.fail_collection(0, "BOOM");
    scope.drain_collection(0);
    assert!(
        scope.advance().is_empty(),
        "a body without compensation, irreversible or high-risk work behaves as it always did"
    );
    assert!(scope.state.scope_saga(&scope.owner).is_none());
}

// --------------------------------------------------------------------------- the scoped events

#[test]
fn a_plan_must_name_real_completed_members_and_be_made_after_the_drain() {
    let mut scope = failed_after_a_completed_iteration();
    let bogus = RunEvent::ScopeCompensationPlanned {
        node: scope.owner.node().clone(),
        cause: FailureCode::parse("X").expect("code"),
        entries: vec![sagat_domain::execution::ScopedPlannedEntry {
            instance: scope.at(1, "ship"),
            after: Vec::new(),
        }],
        irreversible: Vec::new(),
    };
    // `ship` of iteration 1 failed: it never happened, so it has nothing to undo.
    assert!(scope.state.clone().apply(&bogus).is_err());
    // Planned twice is refused.
    scope.advance();
    let again = RunEvent::ScopeCompensationPlanned {
        node: scope.owner.node().clone(),
        cause: FailureCode::parse("X").expect("code"),
        entries: Vec::new(),
        irreversible: Vec::new(),
    };
    assert!(scope.state.clone().apply(&again).is_err());
    // The scope cannot end while a compensation is still open.
    let early = RunEvent::ScopeCompensated {
        node: scope.owner.node().clone(),
        outcome: Outcome::Compensated,
    };
    assert!(scope.state.clone().apply(&early).is_err());
}

#[test]
fn a_collection_cannot_finish_while_its_compensation_is_open() {
    let mut scope = failed_after_a_completed_iteration();
    scope.advance();
    let finish = RunEvent::CollectionFinished {
        node: scope.owner.node().clone(),
        outcome: sagat_domain::execution::CollectionOutcome::Failed(
            FailureCode::parse("PAYMENT_DECLINED").expect("code"),
        ),
        result_digest: None,
    };
    assert!(scope.state.clone().apply(&finish).is_err());
}

// --------------------------------------------------------------- the order, as a pure function

#[test]
fn the_branches_of_a_parallel_node_inside_a_body_are_independent_in_the_order() {
    use std::collections::BTreeSet;

    use sagat_domain::saga::reverse_dependencies;
    use sagat_domain::workflow::JoinPolicy;

    // start -> (a | b) -> finish -> returned: the closed body is defined, though the scoped
    // planner cannot run a fork yet (D388), so the order is proven on the graph itself.
    let nodes = vec![
        step("start", true, false).with_next(id("fan")),
        Node::new(
            id("fan"),
            NodeKind::Parallel {
                branches: vec![id("a"), id("b")],
                join: JoinPolicy::All,
            },
        )
        .with_next(id("finish")),
        step("a", true, false),
        step("b", true, false),
        step("finish", true, false).with_next(id("returned")),
        Node::new(
            id("returned"),
            NodeKind::Return {
                result: expr("steps.start.output"),
            },
        ),
    ];
    let body = BodyDefinition::new(id("line"), id("start"), nodes).expect("closed body");
    let entries: BTreeSet<NodeId> = ["start", "a", "b", "finish"]
        .iter()
        .map(|n| id(n))
        .collect();
    let after = reverse_dependencies(&body, &entries);
    assert_eq!(after[&id("finish")], Vec::<NodeId>::new(), "undone first");
    assert_eq!(
        after[&id("a")],
        vec![id("finish")],
        "a branch waits for what came after the join"
    );
    assert_eq!(after[&id("b")], vec![id("finish")]);
    assert!(
        after[&id("start")].contains(&id("a")) && after[&id("start")].contains(&id("b")),
        "what came before the fork waits for both branches"
    );
    assert!(
        !after[&id("a")].contains(&id("b")) && !after[&id("b")].contains(&id("a")),
        "the branches do not wait for each other"
    );
}

#[test]
fn a_loop_that_timed_out_compensates_and_ends_timed_out_only_if_everything_was_undone() {
    let mut scope = Scope::start(
        loop_definition(vec![step("reserve", true, false)], LoopKind::While),
        "poll",
    );
    scope.enter_loop(LoopKind::While);
    scope.admit_loop(0, "reserve");
    let reserve = scope.at(0, "reserve");
    scope.complete(&reserve);
    scope.checkpoint_loop(0);
    scope.admit_loop(1, "reserve");
    scope.event(RunEvent::RunStopRequested {
        reason: StopReason::TimedOut,
        code: None,
    });
    scope.event(RunEvent::LoopDrainStarted {
        node: scope.owner.node().clone(),
        reason: LoopDrainReason::TimedOut,
    });
    scope.drain_loop(1);
    let chosen = scope.advance();
    let cause = chosen
        .iter()
        .find_map(|addressed| match addressed.event() {
            RunEvent::ScopeCompensationPlanned { cause, .. } => Some(cause.clone()),
            _ => None,
        })
        .expect("planned");
    assert_eq!(cause, FailureCode::run_timed_out());
    let undo = scope.at(0, "reserve~c");
    scope.complete(&undo);
    scope.advance();
    assert_eq!(
        scope.state.final_status_for(StopReason::TimedOut),
        RunStatus::TimedOut
    );
    scope.assert_replays();
}
