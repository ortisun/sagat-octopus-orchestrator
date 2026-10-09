//! Shared fixtures for the execution tests: definition builders and a journaling driver.
#![allow(dead_code, clippy::expect_used)]

use std::time::Duration;

use std::collections::BTreeMap;

use sagat_domain::execution::{
    self, ChildOutcome, Evaluation, ExecutionError, Expiry, Failure, FailureCode, Fence, Heartbeat,
    Progress, Resolution, RunEvent, RunId, RunState, StepStatus, StopReason,
};
use sagat_domain::identity::Slug;
use sagat_domain::saga::{EffectPolicy, OnCompensationFailure, ReconcileVerdict, SagaCancelPolicy};
use sagat_domain::workflow::{
    ActivityName, BackoffStrategy, CancelPolicy, EventType, Expression, JoinPolicy, Node, NodeId,
    NodeKind, RetryPolicy, WorkflowDefinition,
};
use time::OffsetDateTime;

pub const T0: OffsetDateTime = OffsetDateTime::UNIX_EPOCH;

pub fn id(s: &str) -> NodeId {
    NodeId::parse(s).expect("valid node id")
}

/// An id as the journal stores it: a declared id, or a compensation (`x~c`) or reconciliation
/// (`x~r`) derived from one.
pub fn sid(s: &str) -> NodeId {
    NodeId::parse_stored(s).expect("valid stored node id")
}

pub fn expr(s: &str) -> Expression {
    Expression::parse(s).expect("valid expression")
}

pub fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

pub fn activity(node: &str) -> Node {
    activity_with(node, RetryPolicy::DEFAULT)
}

pub fn activity_with(node: &str, retry: RetryPolicy) -> Node {
    Node::new(
        id(node),
        NodeKind::Activity {
            activity: ActivityName::parse(&format!("act.{node}")).expect("name"),
            retry,
            compensate_with: None,
            reconcile_with: None,
            idempotency_template: None,
            secrets: Vec::new(),
        },
    )
}

/// An activity with `attempts` attempts and a fixed 100 ms delay between them.
pub fn flaky(node: &str, attempts: u32) -> Node {
    activity_with(
        node,
        RetryPolicy::new(attempts, ms(100), ms(100), BackoffStrategy::Fixed, false).expect("retry"),
    )
}

pub fn timer(node: &str, secs: u64) -> Node {
    Node::new(
        id(node),
        NodeKind::Timer {
            delay: Duration::from_secs(secs),
        },
    )
}

pub fn wait(node: &str, timeout_secs: u64, on_timeout: Option<&str>) -> Node {
    Node::new(
        id(node),
        NodeKind::WaitEvent {
            event_type: EventType::parse("payment.confirmed").expect("event"),
            correlation: expr("input.paymentId"),
            timeout: Duration::from_secs(timeout_secs),
            on_timeout: on_timeout.map(id),
            lookback: Duration::ZERO,
        },
    )
}

pub fn condition(node: &str, if_true: &str, if_false: &str) -> Node {
    Node::new(
        id(node),
        NodeKind::Condition {
            expression: expr("input.amount > 100"),
            if_true: id(if_true),
            if_false: id(if_false),
        },
    )
}

pub fn parallel(node: &str, branches: &[&str], join: JoinPolicy) -> Node {
    Node::new(
        id(node),
        NodeKind::Parallel {
            branches: branches.iter().map(|b| id(b)).collect(),
            join,
        },
    )
}

/// A child node starting the workflow `child-flow`.
pub fn child(node: &str, policy: CancelPolicy) -> Node {
    Node::new(
        id(node),
        NodeKind::Child {
            workflow: Slug::parse("child-flow").expect("slug"),
            version: None,
            input: None,
            cancel_policy: policy,
        },
    )
}

/// An activity with a compensation (`undo.<node>`) and, when asked, a reconciliation
/// (`lookup.<node>`).
pub fn effect(node: &str, reconcile: bool) -> Node {
    effect_with(node, reconcile, RetryPolicy::DEFAULT)
}

pub fn effect_with(node: &str, reconcile: bool, retry: RetryPolicy) -> Node {
    Node::new(
        id(node),
        NodeKind::Activity {
            activity: ActivityName::parse(&format!("act.{node}")).expect("name"),
            retry,
            compensate_with: Some(ActivityName::parse(&format!("undo.{node}")).expect("name")),
            reconcile_with: reconcile
                .then(|| ActivityName::parse(&format!("lookup.{node}")).expect("name")),
            idempotency_template: None,
            secrets: Vec::new(),
        },
    )
}

/// An activity that is reconciled but has nothing to compensate it with.
pub fn reconciled(node: &str) -> Node {
    Node::new(
        id(node),
        NodeKind::Activity {
            activity: ActivityName::parse(&format!("act.{node}")).expect("name"),
            retry: RetryPolicy::DEFAULT,
            compensate_with: None,
            reconcile_with: Some(ActivityName::parse(&format!("lookup.{node}")).expect("name")),
            idempotency_template: None,
            secrets: Vec::new(),
        },
    )
}

pub fn saga(node: &str, begin: &str) -> Node {
    saga_with(
        node,
        begin,
        OnCompensationFailure::ManualReview,
        SagaCancelPolicy::CompensateCompleted,
    )
}

pub fn saga_with(
    node: &str,
    begin: &str,
    on_compensation_failure: OnCompensationFailure,
    cancel_policy: SagaCancelPolicy,
) -> Node {
    Node::new(
        id(node),
        NodeKind::Saga {
            begin: id(begin),
            on_compensation_failure,
            cancel_policy,
        },
    )
}

pub fn with_effect(node: Node, effect: EffectPolicy) -> Node {
    node.with_effect(effect)
}

pub fn end(node: &str) -> Node {
    Node::new(id(node), NodeKind::End { output: None })
}

pub fn build(entry: &str, nodes: Vec<Node>) -> WorkflowDefinition {
    WorkflowDefinition::new(Slug::parse("demo-flow").expect("slug"), id(entry), nodes)
        .expect("valid definition")
}

/// Drives one run the way the application does and records every journal event.
pub struct Harness {
    pub def: WorkflowDefinition,
    pub state: RunState,
    pub journal: Vec<RunEvent>,
    pub now: OffsetDateTime,
    pub pending: Option<Evaluation>,
    /// The child run reserved for each `child` step, as the engine would record it.
    pub children: BTreeMap<NodeId, RunId>,
}

impl Harness {
    pub fn start(def: WorkflowDefinition) -> Self {
        let (state, progress) =
            execution::start_run(&def, RunId::generate(), T0).expect("run starts");
        let mut harness = Self {
            def,
            state,
            journal: Vec::new(),
            now: T0,
            pending: None,
            children: BTreeMap::new(),
        };
        harness.absorb(progress);
        harness
    }

    fn absorb(&mut self, progress: Progress) {
        self.journal.extend(progress.events);
        self.pending = progress.pending;
    }

    pub fn advance(&mut self, delta: Duration) {
        self.now += delta;
    }

    pub fn status(&self, node: &str) -> StepStatus {
        self.state.step(&sid(node)).expect("step exists").status()
    }

    pub fn has_step(&self, node: &str) -> bool {
        self.state.step(&sid(node)).is_some()
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.journal.iter().map(RunEvent::name).collect()
    }

    /// Leases a ready step; the event is journaled.
    pub fn lease(&mut self, node: &str) -> Fence {
        let (fence, event) = execution::lease_step(&mut self.state, &sid(node)).expect("lease");
        self.journal.push(event);
        fence
    }

    pub fn try_lease(&mut self, node: &str) -> Result<Fence, ExecutionError> {
        let (fence, event) = execution::lease_step(&mut self.state, &sid(node))?;
        self.journal.push(event);
        Ok(fence)
    }

    pub fn heartbeat(&mut self, node: &str, fence: Fence) -> Result<Heartbeat, ExecutionError> {
        let beat = execution::heartbeat(&mut self.state, &sid(node), fence)?;
        self.journal.extend(beat.event.clone());
        Ok(beat)
    }

    pub fn complete(&mut self, node: &str, fence: Fence) -> Result<(), ExecutionError> {
        let progress =
            execution::complete_step(&self.def, &mut self.state, &sid(node), fence, self.now)?;
        self.absorb(progress);
        Ok(())
    }

    pub fn fail(
        &mut self,
        node: &str,
        fence: Fence,
        retryable: bool,
    ) -> Result<(), ExecutionError> {
        let failure = Failure {
            fence,
            code: FailureCode::parse("PAYMENT_DECLINED").expect("code"),
            retryable,
            jitter_sample: 0.0,
        };
        let progress =
            execution::fail_step(&self.def, &mut self.state, &sid(node), failure, self.now)?;
        self.absorb(progress);
        Ok(())
    }

    pub fn expire(
        &mut self,
        node: &str,
        fence: Fence,
        cause: Expiry,
    ) -> Result<(), ExecutionError> {
        let progress = execution::expire_attempt(
            &self.def,
            &mut self.state,
            &sid(node),
            fence,
            cause,
            0.0,
            self.now,
        )?;
        self.absorb(progress);
        Ok(())
    }

    pub fn retry_due(&mut self, node: &str) -> Result<(), ExecutionError> {
        let progress = execution::retry_due(&self.def, &mut self.state, &sid(node), self.now)?;
        self.absorb(progress);
        Ok(())
    }

    pub fn fire_timer(&mut self, node: &str) -> Result<(), ExecutionError> {
        let progress = execution::fire_timer(&self.def, &mut self.state, &sid(node), self.now)?;
        self.absorb(progress);
        Ok(())
    }

    pub fn satisfy_wait(&mut self, node: &str) -> Result<(), ExecutionError> {
        let progress = execution::satisfy_wait(&self.def, &mut self.state, &sid(node), self.now)?;
        self.absorb(progress);
        Ok(())
    }

    pub fn time_out_wait(&mut self, node: &str) -> Result<(), ExecutionError> {
        let progress = execution::time_out_wait(&self.def, &mut self.state, &sid(node), self.now)?;
        self.absorb(progress);
        Ok(())
    }

    pub fn stop(&mut self, reason: StopReason) -> Result<(), ExecutionError> {
        let progress = execution::stop_run(&self.def, &mut self.state, reason, None, self.now)?;
        self.absorb(progress);
        Ok(())
    }

    pub fn pause(&mut self) -> Result<(), ExecutionError> {
        let event = execution::pause_run(&mut self.state)?;
        self.journal.push(event);
        Ok(())
    }

    pub fn resume(&mut self) -> Result<(), ExecutionError> {
        let progress = execution::resume_run(&self.def, &mut self.state, self.now)?;
        self.absorb(progress);
        Ok(())
    }

    /// The reconciliation `node` (`x~r`) answers.
    pub fn reconcile(
        &mut self,
        node: &str,
        fence: Fence,
        verdict: ReconcileVerdict,
    ) -> Result<(), ExecutionError> {
        let progress = execution::complete_reconciliation(
            &self.def,
            &mut self.state,
            &sid(node),
            fence,
            verdict,
            0.0,
            self.now,
        )?;
        self.absorb(progress);
        Ok(())
    }

    pub fn operator_retry(&mut self, node: &str) -> Result<(), ExecutionError> {
        let progress = execution::retry_review(&self.def, &mut self.state, &sid(node), self.now)?;
        self.absorb(progress);
        Ok(())
    }

    pub fn operator_skip(&mut self, node: &str) -> Result<(), ExecutionError> {
        let progress = execution::skip_review(&self.def, &mut self.state, &sid(node), self.now)?;
        self.absorb(progress);
        Ok(())
    }

    pub fn operator_resolve(
        &mut self,
        node: &str,
        outcome: execution::ReviewOutcome,
    ) -> Result<(), ExecutionError> {
        let progress =
            execution::resolve_review(&self.def, &mut self.state, &sid(node), outcome, self.now)?;
        self.absorb(progress);
        Ok(())
    }

    /// Fails an attempt with a given code (the worker's report).
    pub fn fail_with(
        &mut self,
        node: &str,
        fence: Fence,
        code: &str,
        retryable: bool,
    ) -> Result<(), ExecutionError> {
        let failure = Failure {
            fence,
            code: FailureCode::parse(code).expect("code"),
            retryable,
            jitter_sample: 0.0,
        };
        let progress =
            execution::fail_step(&self.def, &mut self.state, &sid(node), failure, self.now)?;
        self.absorb(progress);
        Ok(())
    }

    pub fn resolve(&mut self, resolution: Resolution) -> Result<(), ExecutionError> {
        if let Resolution::Child {
            node,
            child: Ok(child),
        } = &resolution
        {
            self.children.insert(node.clone(), *child);
        }
        let progress = execution::resolve(&self.def, &mut self.state, resolution, self.now)?;
        self.absorb(progress);
        Ok(())
    }

    /// The child run of a `child` step ended.
    pub fn finish_child(
        &mut self,
        node: &str,
        child: RunId,
        outcome: ChildOutcome,
    ) -> Result<(), ExecutionError> {
        let progress = execution::finish_child(
            &self.def,
            &mut self.state,
            &sid(node),
            child,
            outcome,
            self.now,
        )?;
        self.absorb(progress);
        Ok(())
    }

    /// The run reserved for `node`'s child.
    pub fn child_of(&self, node: &str) -> RunId {
        *self.children.get(&sid(node)).expect("a child was reserved")
    }

    /// Resolves the pending evaluation with a fixed answer.
    pub fn answer(&mut self, truth: bool, key: &str) {
        let resolution = match self.pending.clone().expect("an evaluation is pending") {
            Evaluation::Condition { node, .. } => Resolution::Condition {
                node,
                outcome: Ok(truth),
            },
            Evaluation::Correlation { node, .. } => Resolution::Correlation {
                node,
                key: Ok(execution::CorrelationKey::parse(key).expect("key")),
            },
            Evaluation::Child { node, .. } => Resolution::Child {
                node,
                child: Ok(RunId::generate()),
            },
        };
        self.resolve(resolution).expect("resolution applies");
    }

    /// The journal alone must reproduce the state.
    pub fn assert_replays(&self) {
        let replayed =
            RunState::replay(self.state.run_id(), &self.journal).expect("journal replays");
        assert_eq!(
            replayed, self.state,
            "replaying the journal must rebuild the state"
        );
    }
}
