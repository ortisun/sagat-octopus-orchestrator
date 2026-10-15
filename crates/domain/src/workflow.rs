//! Workflow definitions: the typed form of the IR (`workflow.schema.json`).
//!
//! The validating constructors of [`WorkflowDefinition`] prove structural properties before a
//! definition can reach execution. Structured definitions additionally validate named bodies;
//! constructing one does not advertise that its runtime capabilities are available:
//!
//! * every node id is unique and every reference points at an existing node;
//! * every local graph is acyclic and reachable, and named-body references are nonrecursive;
//! * bodies have one explicit return, stay closed and share the definition/nesting budgets;
//! * the branches of a `parallel` node are single-entry regions that do not overlap, do not leave
//!   the region and do not contain a workflow `end`, so a join is well defined;
//! * a `saga` node owns a closed region like a `parallel` node does, so the steps that a failure
//!   inside it must undo are well defined;
//! * delays, timeouts, retry budgets and fan-out are bounded.
//!
//! The compensation and the reconciliation of an activity are *derived nodes* of the definition
//! (`<id>~c`, `<id>~r`): nothing routes to them, but the engine runs them like any other activity,
//! so leases, retries, fencing and idempotency need no second mechanism (feature 003).
//!
//! Wire formats (JSON) are an adapter concern: this module holds no serialization, and expressions
//! are opaque text evaluated by an adapter (`FR-002-001`, ADR-0003).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::Duration;

use crate::execution::{ApprovalPolicy, ExecutionInstanceId};
use crate::identity::{Slug, clip};
use crate::saga::{EffectPolicy, NodeOrigin, OnCompensationFailure, SagaCancelPolicy};
use crate::secrets::{MAX_DECLARED_SECRETS, SecretName};
use crate::triggers::{MAX_TRIGGERS, Trigger};

/// A workflow may have at most this many nodes (matches `nodes.maxItems` in the schema).
pub const MAX_NODES: usize = 10_000;
/// A `parallel` node may fan out to at most this many branches.
pub const MAX_BRANCHES: usize = 256;
/// A switch evaluates no more than this many predicates in declaration order.
pub const MAX_SWITCH_CASES: usize = 32;
/// A workflow may contain at most this many `parallel` nodes. Validating one costs time and memory
/// proportional to its size, so this bounds the cost of a definition to this many times the size of
/// the graph (a document of 650 KB with 5000 nested parallels once cost 17 s and 1.1 GB).
pub const MAX_PARALLELS: usize = 32;
/// A workflow may contain at most this many `saga` nodes. Their regions are computed like those of
/// the `parallel` nodes, so they are bounded the same way.
pub const MAX_SAGAS: usize = 32;
/// Longest timer or wait: a run waiting for months is viable, one waiting for decades is a mistake.
pub const MAX_WAIT: Duration = Duration::from_secs(366 * 24 * 60 * 60);
/// Longest an event may have arrived before a wait started and still be accepted by it.
pub const MAX_LOOKBACK: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// Longest single retry delay.
pub const MAX_RETRY_DELAY: Duration = Duration::from_secs(24 * 60 * 60);
/// Most attempts a retry policy may grant (matches `retry.maxAttempts` in the schema).
pub const MAX_RETRY_ATTEMPTS: u32 = 100;

const MAX_NODE_ID_LEN: usize = 64;
const MAX_NAME_LEN: usize = 128;
const MAX_EXPRESSION_LEN: usize = 2048;

/// Typed errors of the definition model. Messages echo at most a clipped fragment of the input.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkflowError {
    #[error("invalid node id '{0}'")]
    InvalidNodeId(String),
    #[error("invalid activity name '{0}'")]
    InvalidActivityName(String),
    #[error("invalid event type '{0}'")]
    InvalidEventType(String),
    #[error("invalid expression: {0}")]
    InvalidExpression(ExpressionViolation),
    #[error("a workflow needs at least one node")]
    NoNodes,
    #[error("a workflow may have at most {MAX_NODES} nodes, found {0}")]
    TooManyNodes(usize),
    #[error("a workflow may have at most {MAX_PARALLELS} parallel nodes, found {0}")]
    TooManyParallels(usize),
    #[error("a workflow may declare at most {MAX_DECLARED_SECRETS} distinct secrets, found {0}")]
    TooManySecrets(usize),
    #[error("node '{0}' is defined more than once")]
    DuplicateNode(NodeId),
    #[error("entrypoint '{0}' is not a node of the workflow")]
    UnknownEntrypoint(NodeId),
    #[error("node '{from}' refers to '{to}', which does not exist")]
    UnknownTarget { from: NodeId, to: NodeId },
    #[error("the graph has a cycle through node '{0}'")]
    Cycle(NodeId),
    #[error("node '{0}' cannot be reached from the entrypoint")]
    Unreachable(NodeId),
    #[error("invalid retry policy: {0}")]
    InvalidRetry(RetryViolation),
    #[error("node '{node}': a {what} of {millis} ms is outside 1 ms..={max_millis} ms")]
    DurationOutOfRange {
        node: NodeId,
        what: &'static str,
        millis: u128,
        max_millis: u128,
    },
    #[error("parallel node '{0}' needs between 2 and {MAX_BRANCHES} distinct branches")]
    InvalidBranchCount(NodeId),
    #[error("parallel node '{parallel}': branch regions of '{a}' and '{b}' overlap at '{node}'")]
    BranchOverlap {
        parallel: NodeId,
        a: NodeId,
        b: NodeId,
        node: NodeId,
    },
    #[error(
        "parallel node '{parallel}': node '{node}' is reachable both from a branch and from outside it"
    )]
    BranchBoundary { parallel: NodeId, node: NodeId },
    #[error("parallel node '{parallel}': a workflow end ('{node}') cannot sit inside a branch")]
    EndInsideBranch { parallel: NodeId, node: NodeId },
    #[error(
        "saga node '{saga}': node '{node}' is reachable both from inside the saga and from outside it"
    )]
    SagaBoundary { saga: NodeId, node: NodeId },
    #[error("saga node '{saga}': a workflow end ('{node}') cannot sit inside a saga")]
    EndInsideSaga { saga: NodeId, node: NodeId },
    #[error("a workflow may have at most {MAX_SAGAS} saga nodes, found {0}")]
    TooManySagas(usize),
    #[error("node '{0}' is marked irreversible and also has a compensation")]
    IrreversibleWithCompensation(NodeId),
    #[error(
        "node '{0}': effect settings (irreversible, highRisk, compensation retry or timeout) only apply to an activity"
    )]
    EffectOnNonActivity(NodeId),
    #[error("invalid trigger: {0}")]
    InvalidTrigger(&'static str),
    #[error("a workflow may have at most {MAX_TRIGGERS} triggers, found {0}")]
    TooManyTriggers(usize),
    #[error("the workflow declares the same trigger twice")]
    DuplicateTrigger,
    #[error("node '{node}': invalid structured scope ({reason})")]
    InvalidScope { node: NodeId, reason: &'static str },
    #[error("body '{0}' is defined more than once")]
    DuplicateBody(NodeId),
    #[error("node '{node}' refers to unknown body '{body}'")]
    UnknownBody { node: NodeId, body: NodeId },
    #[error("body '{0}' is not referenced by the workflow")]
    UnreachableBody(NodeId),
    #[error("body references contain a cycle through '{0}'")]
    RecursiveBody(NodeId),
    #[error("body nesting exceeds the configured depth of {0}")]
    BodyDepthExceeded(u8),
    #[error("invalid workflow limit: {0}")]
    InvalidWorkflowLimit(&'static str),
    #[error("switch node '{node}': {reason}")]
    InvalidSwitch { node: NodeId, reason: &'static str },
}

/// Why an expression was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ExpressionViolation {
    #[error("must not be empty")]
    Empty,
    #[error("must be at most {MAX_EXPRESSION_LEN} bytes")]
    TooLong,
    #[error("must not contain control characters")]
    ControlCharacter,
}

/// Why a retry policy was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RetryViolation {
    #[error("max attempts must be between 1 and {MAX_RETRY_ATTEMPTS}")]
    Attempts,
    #[error("the initial delay must be between 1 ms and {} s", MAX_RETRY_DELAY.as_secs())]
    InitialDelay,
    #[error("the maximum delay must be at least the initial delay and at most {} s", MAX_RETRY_DELAY.as_secs())]
    MaxDelay,
}

fn is_name_char(c: char, extra: &[char]) -> bool {
    c.is_ascii_alphanumeric() || extra.contains(&c)
}

fn valid_name(value: &str, max: usize, extra: &[char]) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && value.chars().all(|c| is_name_char(c, extra))
}

macro_rules! name_type {
    ($(#[$meta:meta])* $name:ident, $max:expr, $extra:expr, $err:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            /// # Errors
            /// The matching [`WorkflowError`] variant when the value breaks the naming rules.
            pub fn parse(value: &str) -> Result<Self, WorkflowError> {
                if valid_name(value, $max, $extra) {
                    Ok(Self(value.to_owned()))
                } else {
                    Err(WorkflowError::$err(clip(value)))
                }
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

name_type!(
    /// Stable identity of a node inside one workflow: `[A-Za-z0-9_.-]`, starting with a letter or
    /// digit, at most 64 characters. It is part of every step identity, so it must never contain a
    /// path separator.
    NodeId,
    MAX_NODE_ID_LEN,
    &['_', '.', '-'],
    InvalidNodeId
);
impl NodeId {
    /// The character that makes an id a derived one. It is outside the alphabet of declared ids, so
    /// no author can write an id that collides with a derived node.
    const DERIVED: char = '~';

    /// The compensation of this forward node: `<id>~c`.
    #[must_use]
    pub fn compensation(&self) -> Self {
        Self(format!("{}{}c", self.0, Self::DERIVED))
    }

    /// The reconciliation of this forward node: `<id>~r`.
    #[must_use]
    pub fn reconciliation(&self) -> Self {
        Self(format!("{}{}r", self.0, Self::DERIVED))
    }

    /// Whether this is the compensation of a forward node.
    #[must_use]
    pub fn is_compensation(&self) -> bool {
        self.0.ends_with("~c")
    }

    /// Whether this is the reconciliation of a forward node.
    #[must_use]
    pub fn is_reconciliation(&self) -> bool {
        self.0.ends_with("~r")
    }

    /// The forward node this derived id belongs to; `None` for a declared id.
    #[must_use]
    pub fn forward(&self) -> Option<Self> {
        self.0
            .rsplit_once(Self::DERIVED)
            .map(|(base, _)| Self(base.to_owned()))
    }

    /// Whether this id belongs to a derived node (a compensation or a reconciliation).
    #[must_use]
    pub fn is_derived(&self) -> bool {
        self.0.contains(Self::DERIVED)
    }

    /// Reads an id back from a journal or a row: a declared id, or a declared id followed by `~c`
    /// or `~r`. Everything else is rejected, so the derived form cannot be forged from outside.
    ///
    /// # Errors
    /// [`WorkflowError::InvalidNodeId`] unless `value` is one of those forms.
    pub fn parse_stored(value: &str) -> Result<Self, WorkflowError> {
        match value.rsplit_once(Self::DERIVED) {
            None => Self::parse(value),
            Some((base, "c" | "r")) => {
                Self::parse(base)?;
                Ok(Self(value.to_owned()))
            }
            Some(_) => Err(WorkflowError::InvalidNodeId(clip(value))),
        }
    }
}

name_type!(
    /// Name of an activity a worker advertises, e.g. `payment.validate`.
    ActivityName,
    MAX_NAME_LEN,
    &['_', '.', ':', '-'],
    InvalidActivityName
);
name_type!(
    /// Type of an event a `waitEvent` node subscribes to, e.g. `payment.confirmed`.
    EventType,
    MAX_NAME_LEN,
    &['_', '.', ':', '-'],
    InvalidEventType
);

/// Opaque expression text (CEL in the IR). The domain does not interpret it; an adapter evaluates
/// it with no side effects. It is stored with the version, so its meaning is pinned to the version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expression(String);

impl Expression {
    /// # Errors
    /// [`WorkflowError::InvalidExpression`] when empty, too long or containing control characters.
    pub fn parse(value: &str) -> Result<Self, WorkflowError> {
        let violation = if value.trim().is_empty() {
            Some(ExpressionViolation::Empty)
        } else if value.len() > MAX_EXPRESSION_LEN {
            Some(ExpressionViolation::TooLong)
        } else if value.chars().any(char::is_control) {
            Some(ExpressionViolation::ControlCharacter)
        } else {
            None
        };
        violation.map_or_else(
            || Ok(Self(value.to_owned())),
            |v| Err(WorkflowError::InvalidExpression(v)),
        )
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// SHA-256 digest of a canonical definition, or of a payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Digest([u8; 32]);

impl Digest {
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Lower-case hex form (64 characters), the form that is stored and shown.
    #[must_use]
    pub fn to_hex(&self) -> String {
        use fmt::Write;
        self.0.iter().fold(String::with_capacity(64), |mut out, b| {
            // Writing to a String cannot fail.
            let _ = write!(out, "{b:02x}");
            out
        })
    }

    /// # Errors
    /// `None` unless `hex` is exactly 64 hex digits.
    #[must_use]
    pub fn from_hex(hex: &str) -> Option<Self> {
        if hex.len() != 64 || !hex.is_ascii() {
            return None;
        }
        let mut out = [0u8; 32];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
        }
        Some(Self(out))
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// How the delay between attempts grows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackoffStrategy {
    Fixed,
    Exponential,
}

/// Retry budget and backoff of one node (`FR-002-005`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    max_attempts: u32,
    initial_delay: Duration,
    max_delay: Duration,
    strategy: BackoffStrategy,
    jitter: bool,
}

impl RetryPolicy {
    /// What a node gets when the definition says nothing: three attempts, exponential from one
    /// second up to one minute, with jitter. A decision, recorded in `specs/002-durable-execution/research.md` (D26).
    pub const DEFAULT: Self = Self {
        max_attempts: 3,
        initial_delay: Duration::from_secs(1),
        max_delay: Duration::from_secs(60),
        strategy: BackoffStrategy::Exponential,
        jitter: true,
    };

    /// What a compensation gets when the author says nothing: five attempts, exponential from two
    /// seconds up to five minutes, with jitter. A compensation that keeps failing is a decision for
    /// an operator, so it is given longer than a forward step to recover (feature 003, D63).
    pub const COMPENSATION: Self = Self {
        max_attempts: 5,
        initial_delay: Duration::from_secs(2),
        max_delay: Duration::from_secs(300),
        strategy: BackoffStrategy::Exponential,
        jitter: true,
    };

    /// What a reconciliation gets: ten attempts in all, shared by every round of reconciling one
    /// step, exponential from one second up to thirty. A lookup is cheap and must not give up
    /// early, because the alternative to an answer is an operator.
    pub const RECONCILIATION: Self = Self {
        max_attempts: 10,
        initial_delay: Duration::from_secs(1),
        max_delay: Duration::from_secs(30),
        strategy: BackoffStrategy::Exponential,
        jitter: true,
    };

    /// # Errors
    /// [`WorkflowError::InvalidRetry`] when any bound is violated.
    pub fn new(
        max_attempts: u32,
        initial_delay: Duration,
        max_delay: Duration,
        strategy: BackoffStrategy,
        jitter: bool,
    ) -> Result<Self, WorkflowError> {
        let violation = if !(1..=MAX_RETRY_ATTEMPTS).contains(&max_attempts) {
            Some(RetryViolation::Attempts)
        } else if initial_delay < Duration::from_millis(1) || initial_delay > MAX_RETRY_DELAY {
            Some(RetryViolation::InitialDelay)
        } else if max_delay < initial_delay || max_delay > MAX_RETRY_DELAY {
            Some(RetryViolation::MaxDelay)
        } else {
            None
        };
        violation.map_or(
            Ok(Self {
                max_attempts,
                initial_delay,
                max_delay,
                strategy,
                jitter,
            }),
            |v| Err(WorkflowError::InvalidRetry(v)),
        )
    }

    /// A policy that never retries.
    #[must_use]
    pub const fn no_retry() -> Self {
        Self {
            max_attempts: 1,
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(1),
            strategy: BackoffStrategy::Fixed,
            jitter: false,
        }
    }

    #[must_use]
    pub const fn max_attempts(&self) -> u32 {
        self.max_attempts
    }

    #[must_use]
    pub const fn initial_delay(&self) -> Duration {
        self.initial_delay
    }

    #[must_use]
    pub const fn max_delay(&self) -> Duration {
        self.max_delay
    }

    #[must_use]
    pub const fn strategy(&self) -> BackoffStrategy {
        self.strategy
    }

    #[must_use]
    pub const fn jitter(&self) -> bool {
        self.jitter
    }

    /// Whether another attempt may follow `attempts_used` attempts.
    #[must_use]
    pub const fn has_attempts_left(&self, attempts_used: u32) -> bool {
        attempts_used < self.max_attempts
    }

    /// Delay before the attempt that follows the failed attempt number `failed_attempt` (1-based).
    ///
    /// `jitter_sample` is a uniform sample in `[0, 1)` drawn by the caller, so this function stays
    /// pure. With jitter the delay lies in `[base/2, base)` ("equal jitter"): it spreads retries
    /// without ever collapsing to zero. The result never exceeds `max_delay`.
    #[must_use]
    pub fn delay_after(&self, failed_attempt: u32, jitter_sample: f64) -> Duration {
        let base = match self.strategy {
            BackoffStrategy::Fixed => self.initial_delay,
            BackoffStrategy::Exponential => {
                let shift = failed_attempt.saturating_sub(1).min(40);
                let millis = self
                    .initial_delay
                    .as_millis()
                    .saturating_mul(1u128 << shift);
                let capped = millis.min(self.max_delay.as_millis());
                Duration::from_millis(u64::try_from(capped).unwrap_or(u64::MAX))
            }
        };
        // `initial_delay <= max_delay` is checked in `new`, so `base <= max_delay` already.
        if !self.jitter {
            return base;
        }
        let sample = if jitter_sample.is_finite() {
            jitter_sample.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let half = base / 2;
        // `sample < 1` by contract; a clamped 1.0 still stays within `base`.
        (half + half.mul_f64(sample)).min(base)
    }
}

/// How a `parallel` node decides it is finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinPolicy {
    /// Every branch must succeed; the first branch failure fails the node and cancels the others.
    All,
    /// The first branch to succeed wins; the others are cancelled. If every branch fails, the node
    /// fails.
    Any,
}

/// What happens to a child run when the run that started it is stopped (`FR-002-009`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CancelPolicy {
    /// The child is cancelled; the parent does not wait for it to settle. The default.
    Propagate,
    /// The child is cancelled and the parent waits until it has actually settled before it finishes.
    RequestCancel,
    /// The child keeps running on its own.
    Detach,
}

impl CancelPolicy {
    /// The name stored and shown (`PROPAGATE`, `REQUEST_CANCEL`, `DETACH`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Propagate => "PROPAGATE",
            Self::RequestCancel => "REQUEST_CANCEL",
            Self::Detach => "DETACH",
        }
    }

    /// Whether stopping the parent stops the child.
    #[must_use]
    pub const fn cascades(self) -> bool {
        !matches!(self, Self::Detach)
    }
}

impl std::str::FromStr for CancelPolicy {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, ()> {
        match value {
            "PROPAGATE" => Ok(Self::Propagate),
            "REQUEST_CANCEL" => Ok(Self::RequestCancel),
            "DETACH" => Ok(Self::Detach),
            _ => Err(()),
        }
    }
}

/// A named, ordered predicate and its target within one switch node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchCase {
    id: NodeId,
    when: Expression,
    target: NodeId,
}

impl SwitchCase {
    #[must_use]
    pub const fn new(id: NodeId, when: Expression, target: NodeId) -> Self {
        Self { id, when, target }
    }

    #[must_use]
    pub const fn id(&self) -> &NodeId {
        &self.id
    }

    #[must_use]
    pub const fn when(&self) -> &Expression {
        &self.when
    }

    #[must_use]
    pub const fn target(&self) -> &NodeId {
        &self.target
    }
}

/// What a definition node does. Some kinds require a closed body and runtime support.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    /// Durable external task executed by a worker.
    Activity {
        activity: ActivityName,
        retry: RetryPolicy,
        /// Recorded for the saga engine (feature 003); not executed by this version.
        compensate_with: Option<ActivityName>,
        /// Recorded for the saga engine (feature 003); not executed by this version.
        reconcile_with: Option<ActivityName>,
        /// Template of the idempotency key handed to the worker; `None` means the default key.
        idempotency_template: Option<Expression>,
        /// Secrets the worker may ask for while it holds a lease on this step (FR-009-002). Names
        /// only: a value is never part of a definition, a journal or a lease.
        secrets: Vec<SecretName>,
    },
    /// Waits for a delay without holding any process.
    Timer { delay: Duration },
    /// Waits for a correlated event, with a deadline.
    WaitEvent {
        event_type: EventType,
        correlation: Expression,
        timeout: Duration,
        on_timeout: Option<NodeId>,
        /// How long before the wait started an event of this type and correlation may have arrived
        /// and still wake it (zero: only an event that arrives while it waits does). The event may
        /// come from any source; it is delivered once.
        lookback: Duration,
    },
    /// Chooses between two successors by a side-effect-free expression.
    Condition {
        expression: Expression,
        if_true: NodeId,
        if_false: NodeId,
    },
    /// Chooses the first true case or its mandatory default.
    Switch {
        cases: Vec<SwitchCase>,
        default: NodeId,
    },
    /// Requests one authorized human decision, then follows its recorded outcome.
    Approval {
        policy: ApprovalPolicy,
        on_approve: NodeId,
        on_reject: NodeId,
        on_expire: NodeId,
    },
    /// Fans out to `branches` and joins them according to `join`.
    Parallel {
        branches: Vec<NodeId>,
        join: JoinPolicy,
    },
    /// Starts another workflow of the same namespace as a child run and waits for it, holding no
    /// process while it does (`FR-002-009`).
    Child {
        /// The child workflow's name, resolved in the parent's namespace when the child starts.
        workflow: Slug,
        /// A pinned version number; the workflow's active version when absent.
        version: Option<u32>,
        /// Evaluated over the parent's data into the child's input; the parent's own input when
        /// absent.
        input: Option<Expression>,
        cancel_policy: CancelPolicy,
    },
    /// A transactional scope: everything reachable from `begin` is the saga. If a step inside it
    /// fails for good, the steps that completed are compensated in reverse dependency order
    /// (feature 003). When the scope completes, control goes to `next`.
    Saga {
        begin: NodeId,
        on_compensation_failure: OnCompensationFailure,
        /// What a cancellation or a deadline does to the saga.
        cancel_policy: SagaCancelPolicy,
    },
    /// Invokes one closed body for each captured collection element.
    ForEach {
        items: Expression,
        body: NodeId,
        result: Expression,
        max_items: u32,
        max_concurrency: u16,
    },
    /// Evaluates the condition before each sequential body iteration.
    While {
        condition: Expression,
        body: NodeId,
        initial: Expression,
        carry: Expression,
        max_iterations: u32,
        delay: Option<Duration>,
    },
    /// Executes one sequential body iteration before evaluating the stop condition.
    RepeatUntil {
        condition: Expression,
        body: NodeId,
        initial: Expression,
        carry: Expression,
        max_iterations: u32,
        delay: Option<Duration>,
    },
    /// Returns a value from a named body; never a workflow terminal.
    Return { result: Expression },
    /// Closes the workflow successfully.
    End { output: Option<Expression> },
}

impl NodeKind {
    /// Short stable name used in journals and metrics (never user data).
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Activity { .. } => "activity",
            Self::Timer { .. } => "timer",
            Self::WaitEvent { .. } => "waitEvent",
            Self::Condition { .. } => "condition",
            Self::Switch { .. } => "switch",
            Self::Approval { .. } => "approval",
            Self::Parallel { .. } => "parallel",
            Self::Child { .. } => "child",
            Self::Saga { .. } => "saga",
            Self::End { .. } => "end",
            Self::ForEach { .. } => "forEach",
            Self::While { .. } => "while",
            Self::RepeatUntil { .. } => "repeatUntil",
            Self::Return { .. } => "return",
        }
    }
}

/// One node of the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    id: NodeId,
    kind: NodeKind,
    next: Option<NodeId>,
    on_error: Option<NodeId>,
    timeout: Option<Duration>,
    effect: EffectPolicy,
    origin: NodeOrigin,
}

impl Node {
    #[must_use]
    pub const fn new(id: NodeId, kind: NodeKind) -> Self {
        Self {
            id,
            kind,
            next: None,
            on_error: None,
            timeout: None,
            effect: EffectPolicy::NONE,
            origin: NodeOrigin::Declared,
        }
    }

    /// What the author says about the effect of this activity (irreversible, high risk, how its
    /// compensation retries).
    #[must_use]
    pub const fn with_effect(mut self, effect: EffectPolicy) -> Self {
        self.effect = effect;
        self
    }

    /// Successor after a success.
    #[must_use]
    pub fn with_next(mut self, next: NodeId) -> Self {
        self.next = Some(next);
        self
    }

    /// Successor after a failure that exhausted the retry budget.
    #[must_use]
    pub fn with_on_error(mut self, on_error: NodeId) -> Self {
        self.on_error = Some(on_error);
        self
    }

    /// Longest an attempt may take before it counts as failed.
    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    #[must_use]
    pub const fn id(&self) -> &NodeId {
        &self.id
    }

    #[must_use]
    pub const fn kind(&self) -> &NodeKind {
        &self.kind
    }

    #[must_use]
    pub const fn next(&self) -> Option<&NodeId> {
        self.next.as_ref()
    }

    #[must_use]
    pub const fn on_error(&self) -> Option<&NodeId> {
        self.on_error.as_ref()
    }

    #[must_use]
    pub const fn timeout(&self) -> Option<Duration> {
        self.timeout
    }

    #[must_use]
    pub const fn effect(&self) -> &EffectPolicy {
        &self.effect
    }

    /// Whether the author wrote this node or the definition derived it from `compensateWith` or
    /// `reconcileWith`.
    #[must_use]
    pub const fn origin(&self) -> &NodeOrigin {
        &self.origin
    }

    /// The retry policy that governs this node (only activities retry).
    #[must_use]
    pub const fn retry_policy(&self) -> RetryPolicy {
        match &self.kind {
            NodeKind::Activity { retry, .. } => *retry,
            _ => RetryPolicy::no_retry(),
        }
    }

    /// The secrets this node declares (only an activity can).
    #[must_use]
    pub fn secrets(&self) -> &[SecretName] {
        match &self.kind {
            NodeKind::Activity { secrets, .. } => secrets,
            _ => &[],
        }
    }

    /// Every node this node can hand control to, in a stable order.
    #[must_use]
    pub fn successors(&self) -> Vec<&NodeId> {
        let mut out: Vec<&NodeId> = Vec::new();
        match &self.kind {
            NodeKind::Condition {
                if_true, if_false, ..
            } => {
                out.push(if_true);
                out.push(if_false);
            }
            NodeKind::Switch { cases, default } => {
                out.extend(cases.iter().map(SwitchCase::target));
                out.push(default);
            }
            NodeKind::Approval {
                on_approve,
                on_reject,
                on_expire,
                ..
            } => {
                out.push(on_approve);
                out.push(on_reject);
                out.push(on_expire);
            }
            NodeKind::Parallel { branches, .. } => out.extend(branches.iter()),
            NodeKind::Saga { begin, .. } => out.push(begin),
            NodeKind::WaitEvent { on_timeout, .. } => out.extend(on_timeout.iter()),
            _ => {}
        }
        out.extend(self.next.iter());
        out.extend(self.on_error.iter());
        out
    }
}

/// A non-fatal observation made while validating a definition, reported to the author.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefinitionWarning {
    /// `compensateWith` on a node that is not inside a `saga`: nothing starts a compensation there,
    /// so it will never run.
    CompensationOutsideSaga { node: NodeId },
    /// A child node starts the workflow it belongs to. Legal (a condition can stop the recursion),
    /// but the depth limit ends it at the latest.
    RecursiveChild { node: NodeId },
}

impl fmt::Display for DefinitionWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CompensationOutsideSaga { node } => write!(
                f,
                "node '{node}': compensateWith only runs when the node is inside a saga; here it never will"
            ),
            Self::RecursiveChild { node } => write!(
                f,
                "node '{node}': starts the workflow it belongs to; the nesting depth limit ends the recursion"
            ),
        }
    }
}

/// Shared lifetime admission and structural nesting ceilings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkflowLimits {
    max_instances: u32,
    max_depth: u8,
}

impl WorkflowLimits {
    /// # Errors
    /// Rejects an instance ceiling outside 1..=100000 or depth outside 1..=8.
    pub fn new(max_instances: u32, max_depth: u8) -> Result<Self, WorkflowError> {
        if !(1..=100_000).contains(&max_instances) {
            return Err(WorkflowError::InvalidWorkflowLimit(
                "maxInstances must be in 1..=100000",
            ));
        }
        if !(1..=8).contains(&max_depth) {
            return Err(WorkflowError::InvalidWorkflowLimit(
                "maxDepth must be in 1..=8",
            ));
        }
        Ok(Self {
            max_instances,
            max_depth,
        })
    }

    #[must_use]
    pub const fn max_instances(self) -> u32 {
        self.max_instances
    }

    #[must_use]
    pub const fn max_depth(self) -> u8 {
        self.max_depth
    }
}

impl Default for WorkflowLimits {
    fn default() -> Self {
        Self {
            max_instances: 100_000,
            max_depth: 8,
        }
    }
}

/// A locally validated closed body. References to other bodies are checked with the workflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyDefinition {
    name: NodeId,
    graph: DefinitionGraph,
}

impl BodyDefinition {
    /// # Errors
    /// Rejects graph cycles/escapes, workflow ends and missing/multiple or nonterminal returns.
    pub fn new(name: NodeId, entrypoint: NodeId, nodes: Vec<Node>) -> Result<Self, WorkflowError> {
        Ok(Self {
            name,
            graph: DefinitionGraph::new(entrypoint, nodes, GraphKind::Body)?,
        })
    }

    #[must_use]
    pub const fn name(&self) -> &NodeId {
        &self.name
    }

    #[must_use]
    pub const fn entrypoint(&self) -> &NodeId {
        &self.graph.entrypoint
    }

    #[must_use]
    pub fn node(&self, id: &NodeId) -> Option<&Node> {
        self.graph.nodes.get(id)
    }

    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.graph
            .nodes
            .values()
            .filter(|node| node.origin == NodeOrigin::Declared)
    }

    pub fn all_nodes(&self) -> impl Iterator<Item = &Node> {
        self.graph.nodes.values()
    }

    #[must_use]
    pub fn enclosing_scope(&self, node: &NodeId) -> Option<&Scope> {
        self.graph.scopes.get(node)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DefinitionGraph {
    entrypoint: NodeId,
    nodes: BTreeMap<NodeId, Node>,
    declared: usize,
    regions: BTreeMap<NodeId, Vec<BTreeSet<NodeId>>>,
    saga_regions: BTreeMap<NodeId, BTreeSet<NodeId>>,
    scopes: BTreeMap<NodeId, Scope>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GraphKind {
    Legacy,
    Structured,
    Body,
}

/// An immutable workflow graph and, for a structured revision, its closed named bodies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowDefinition {
    name: Slug,
    graph: DefinitionGraph,
    declared: usize,
    bodies: BTreeMap<NodeId, BodyDefinition>,
    limits: WorkflowLimits,
    structured: bool,
    triggers: Vec<Trigger>,
}

/// A region of the graph that decides what a failure inside it means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// A branch of this `parallel` node: its join decides.
    Parallel(NodeId),
    /// The body of this `saga` node: the saga compensates.
    Saga(NodeId),
}

impl Scope {
    /// The node that owns the region.
    #[must_use]
    pub const fn owner(&self) -> &NodeId {
        match self {
            Self::Parallel(node) | Self::Saga(node) => node,
        }
    }
}

impl WorkflowDefinition {
    /// Validates an existing root-only definition without changing its accepted forms.
    ///
    /// # Errors
    /// Returns the first violated graph rule or a structured kind in a legacy definition.
    pub fn new(name: Slug, entrypoint: NodeId, nodes: Vec<Node>) -> Result<Self, WorkflowError> {
        let graph = DefinitionGraph::new(entrypoint, nodes, GraphKind::Legacy)?;
        Ok(Self {
            name,
            declared: graph.declared,
            graph,
            bodies: BTreeMap::new(),
            limits: WorkflowLimits::default(),
            structured: false,
            triggers: Vec::new(),
        })
    }

    /// Validates a structured definition without enabling its execution or publication.
    ///
    /// # Errors
    /// Rejects invalid graphs, missing/recursive/unreachable bodies, depth or aggregate ceilings.
    pub fn new_structured(
        name: Slug,
        entrypoint: NodeId,
        nodes: Vec<Node>,
        bodies: Vec<BodyDefinition>,
        limits: WorkflowLimits,
    ) -> Result<Self, WorkflowError> {
        let graph = DefinitionGraph::new(entrypoint, nodes, GraphKind::Structured)?;
        let mut named = BTreeMap::new();
        let mut total = graph.declared;
        for body in bodies {
            total = total.saturating_add(body.graph.declared);
            if total > MAX_NODES {
                return Err(WorkflowError::TooManyNodes(total));
            }
            let name = body.name.clone();
            if named.insert(name.clone(), body).is_some() {
                return Err(WorkflowError::DuplicateBody(name));
            }
        }
        check_body_references(&graph, &named, limits)?;
        let definition = Self {
            name,
            graph,
            declared: total,
            bodies: named,
            limits,
            structured: true,
            triggers: Vec::new(),
        };
        definition.check_aggregate_limits()?;
        Ok(definition)
    }

    #[must_use]
    pub const fn is_structured(&self) -> bool {
        self.structured
    }

    #[must_use]
    pub const fn limits(&self) -> WorkflowLimits {
        self.limits
    }

    #[must_use]
    pub fn body(&self, name: &NodeId) -> Option<&BodyDefinition> {
        self.bodies.get(name)
    }

    pub fn bodies(&self) -> impl Iterator<Item = &BodyDefinition> {
        self.bodies.values()
    }

    fn graphs(&self) -> impl Iterator<Item = &DefinitionGraph> {
        std::iter::once(&self.graph).chain(self.bodies.values().map(|body| &body.graph))
    }

    fn check_aggregate_limits(&self) -> Result<(), WorkflowError> {
        let parallels: usize = self.graphs().map(|graph| graph.regions.len()).sum();
        if parallels > MAX_PARALLELS {
            return Err(WorkflowError::TooManyParallels(parallels));
        }
        let sagas: usize = self.graphs().map(|graph| graph.saga_regions.len()).sum();
        if sagas > MAX_SAGAS {
            return Err(WorkflowError::TooManySagas(sagas));
        }
        let secrets = self.declared_secrets().len();
        if secrets > MAX_DECLARED_SECRETS {
            return Err(WorkflowError::TooManySecrets(secrets));
        }
        Ok(())
    }

    /// The definition with `triggers` (feature 004): each must be well formed, there are at most
    /// [`MAX_TRIGGERS`] and none is about the same thing as another (a filter does not tell two apart).
    ///
    /// # Errors
    /// [`WorkflowError::InvalidTrigger`], [`WorkflowError::TooManyTriggers`] or
    /// [`WorkflowError::DuplicateTrigger`].
    pub fn with_triggers(mut self, triggers: Vec<Trigger>) -> Result<Self, WorkflowError> {
        if triggers.len() > MAX_TRIGGERS {
            return Err(WorkflowError::TooManyTriggers(triggers.len()));
        }
        for (i, trigger) in triggers.iter().enumerate() {
            trigger.validate()?;
            if triggers[..i]
                .iter()
                .any(|earlier| earlier.is_same_trigger(trigger))
            {
                return Err(WorkflowError::DuplicateTrigger);
            }
        }
        self.triggers = triggers;
        Ok(self)
    }

    /// What starts a run other than somebody asking.
    #[must_use]
    pub fn triggers(&self) -> &[Trigger] {
        &self.triggers
    }

    #[must_use]
    pub const fn name(&self) -> &Slug {
        &self.name
    }

    #[must_use]
    pub const fn entrypoint(&self) -> &NodeId {
        &self.graph.entrypoint
    }

    #[must_use]
    pub fn node(&self, id: &NodeId) -> Option<&Node> {
        self.graph.nodes.get(id)
    }

    /// Resolves a logical execution through its declared collection bodies. Local node names
    /// may repeat in different bodies; each scope owner selects the next graph.
    #[must_use]
    pub fn node_for_instance(&self, instance: &ExecutionInstanceId) -> Option<&Node> {
        let mut graph = &self.graph;
        for segment in instance.path().segments() {
            let owner = graph.nodes.get(segment.scope_node())?;
            let (body, max_iterations) = match owner.kind() {
                NodeKind::ForEach {
                    body, max_items, ..
                } => (body, *max_items),
                NodeKind::While {
                    body,
                    max_iterations,
                    ..
                }
                | NodeKind::RepeatUntil {
                    body,
                    max_iterations,
                    ..
                } => (body, *max_iterations),
                _ => return None,
            };
            if segment.iteration_index() >= max_iterations {
                return None;
            }
            graph = &self.bodies.get(body)?.graph;
        }
        graph.nodes.get(instance.node())
    }

    /// The root graph's authored nodes, ordered by local id; bodies have separate local graphs.
    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.graph
            .nodes
            .values()
            .filter(|node| node.origin == NodeOrigin::Declared)
    }

    /// Root graph nodes, including derived companions, ordered by local id.
    /// Body nodes are obtained from their named body so equal local ids never collide.
    pub fn all_nodes(&self) -> impl Iterator<Item = &Node> {
        self.graph.nodes.values()
    }

    /// Every secret any activity of the definition declares, once each, in name order.
    #[must_use]
    pub fn declared_secrets(&self) -> BTreeSet<&SecretName> {
        self.graphs()
            .flat_map(|graph| graph.nodes.values())
            .filter(|node| node.origin == NodeOrigin::Declared)
            .flat_map(Node::secrets)
            .collect()
    }

    /// The artifact of every script any node runs, once each: the digest in the name of the
    /// activity a `script` node becomes.
    #[must_use]
    pub fn script_digests(&self) -> BTreeSet<crate::scripts::Digest> {
        self.graphs()
            .flat_map(|graph| graph.nodes.values())
            .filter(|node| node.origin == NodeOrigin::Declared)
            .filter_map(|node| match &node.kind {
                NodeKind::Activity { activity, .. } => {
                    crate::scripts::digest_of_activity(activity.as_str())
                }
                _ => None,
            })
            .collect()
    }

    /// How many nodes the author wrote.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.declared
    }

    /// How many nodes the engine can run: those, and the compensations and reconciliations derived
    /// from them.
    #[must_use]
    pub fn total_len(&self) -> usize {
        self.graphs().map(|graph| graph.nodes.len()).sum()
    }

    /// A conservative estimate of the memory this definition occupies, in bytes, so caches can be
    /// bounded by weight and not just by count.
    #[must_use]
    pub fn footprint(&self) -> usize {
        // A node: the struct, its id and edges, expressions; generous on purpose.
        const PER_NODE: usize = 640;
        // A region entry: a `NodeId` (a `String`) in a tree set.
        const PER_REGION_ENTRY: usize = 96;
        let region_entries: usize = self
            .graph
            .regions
            .values()
            .map(|branches| branches.iter().map(BTreeSet::len).sum::<usize>())
            .sum();
        let saga_entries: usize = self.graph.saga_regions.values().map(BTreeSet::len).sum();
        let body_regions: usize = self
            .bodies
            .values()
            .map(|body| {
                body.graph
                    .regions
                    .values()
                    .map(|branches| branches.iter().map(BTreeSet::len).sum::<usize>())
                    .sum::<usize>()
                    + body
                        .graph
                        .saga_regions
                        .values()
                        .map(BTreeSet::len)
                        .sum::<usize>()
                    + body.graph.scopes.len()
            })
            .sum();
        4096 + self.total_len() * PER_NODE
            + (region_entries + saga_entries + self.graph.scopes.len() + body_regions)
                * PER_REGION_ENTRY
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.graph.nodes.is_empty()
    }

    /// The node sets of the branches of a `parallel` node, aligned with its `branches`.
    #[must_use]
    pub fn branch_regions(&self, parallel: &NodeId) -> Option<&[BTreeSet<NodeId>]> {
        self.graph.regions.get(parallel).map(Vec::as_slice)
    }

    /// The `parallel` node whose branch directly contains `node`: the one whose join decides what
    /// a failure of `node` means. `None` when the innermost region around `node` is a saga, or when
    /// there is none.
    #[must_use]
    pub fn enclosing_parallel(&self, node: &NodeId) -> Option<&NodeId> {
        match self.graph.scopes.get(node) {
            Some(Scope::Parallel(parallel)) => Some(parallel),
            _ => None,
        }
    }

    /// The innermost region (branch of a `parallel` node, or body of a `saga` node) around `node`.
    #[must_use]
    pub fn enclosing_scope(&self, node: &NodeId) -> Option<&Scope> {
        self.graph.scopes.get(node)
    }

    /// Every region around `node`, innermost first.
    pub fn scope_chain<'a>(&'a self, node: &NodeId) -> impl Iterator<Item = &'a Scope> + use<'a> {
        let mut current = self.graph.scopes.get(node);
        std::iter::from_fn(move || {
            let scope = current?;
            current = self.graph.scopes.get(scope.owner());
            Some(scope)
        })
    }

    /// The node set of the body of a `saga` node.
    #[must_use]
    pub fn saga_region(&self, saga: &NodeId) -> Option<&BTreeSet<NodeId>> {
        self.graph.saga_regions.get(saga)
    }

    /// The `saga` nodes, ordered by id.
    pub fn sagas(&self) -> impl Iterator<Item = &NodeId> {
        self.graph.saga_regions.keys()
    }

    /// Observations for the author; none of them makes the definition invalid.
    #[must_use]
    pub fn warnings(&self) -> Vec<DefinitionWarning> {
        let mut out = Vec::new();
        for node in self.nodes() {
            match node.kind() {
                NodeKind::Activity {
                    compensate_with: Some(_),
                    ..
                } if !self
                    .scope_chain(node.id())
                    .any(|scope| matches!(scope, Scope::Saga(_))) =>
                {
                    out.push(DefinitionWarning::CompensationOutsideSaga {
                        node: node.id().clone(),
                    });
                }
                NodeKind::Child { workflow, .. } if *workflow == self.name => {
                    out.push(DefinitionWarning::RecursiveChild {
                        node: node.id().clone(),
                    });
                }
                _ => {}
            }
        }
        out
    }
}

impl DefinitionGraph {
    fn new(entrypoint: NodeId, nodes: Vec<Node>, kind: GraphKind) -> Result<Self, WorkflowError> {
        if nodes.is_empty() {
            return Err(WorkflowError::NoNodes);
        }
        if nodes.len() > MAX_NODES {
            return Err(WorkflowError::TooManyNodes(nodes.len()));
        }
        let mut map = BTreeMap::new();
        for node in nodes {
            let id = node.id.clone();
            if map.insert(id.clone(), node).is_some() {
                return Err(WorkflowError::DuplicateNode(id));
            }
        }
        if !map.contains_key(&entrypoint) {
            return Err(WorkflowError::UnknownEntrypoint(entrypoint));
        }
        for node in map.values() {
            check_node(node)?;
            check_graph_kind(node, kind)?;
            for target in node.successors() {
                if !map.contains_key(target) {
                    return Err(WorkflowError::UnknownTarget {
                        from: node.id.clone(),
                        to: target.clone(),
                    });
                }
            }
        }
        // Before anything proportional to (parallels x nodes) is computed.
        let parallels = map
            .values()
            .filter(|n| matches!(n.kind(), NodeKind::Parallel { .. }))
            .count();
        if parallels > MAX_PARALLELS {
            return Err(WorkflowError::TooManyParallels(parallels));
        }
        let sagas = map
            .values()
            .filter(|n| matches!(n.kind(), NodeKind::Saga { .. }))
            .count();
        if sagas > MAX_SAGAS {
            return Err(WorkflowError::TooManySagas(sagas));
        }
        let declared = map
            .values()
            .flat_map(|node| node.secrets())
            .collect::<BTreeSet<_>>()
            .len();
        if declared > MAX_DECLARED_SECRETS {
            return Err(WorkflowError::TooManySecrets(declared));
        }
        check_acyclic(&map)?;
        check_reachable(&map, &entrypoint)?;
        let regions = parallel_regions(&map)?;
        let saga_regions = saga_regions(&map)?;
        let scopes = innermost_scopes(&regions, &saga_regions);
        if kind == GraphKind::Body {
            check_body_return(&map, &entrypoint, &scopes)?;
        }
        let declared = map.len();
        // Only now, so that nothing above had to know about them: the nodes that run compensations
        // and reconciliations.
        for derived in derive_companions(&map) {
            map.insert(derived.id.clone(), derived);
        }
        Ok(Self {
            entrypoint,
            nodes: map,
            declared,
            regions,
            saga_regions,
            scopes,
        })
    }
}

fn duration_in_range(
    node: &NodeId,
    what: &'static str,
    value: Duration,
) -> Result<(), WorkflowError> {
    if value < Duration::from_millis(1) || value > MAX_WAIT {
        return Err(WorkflowError::DurationOutOfRange {
            node: node.clone(),
            what,
            millis: value.as_millis(),
            max_millis: MAX_WAIT.as_millis(),
        });
    }
    Ok(())
}

fn scope_error(node: &NodeId, reason: &'static str) -> WorkflowError {
    WorkflowError::InvalidScope {
        node: node.clone(),
        reason,
    }
}

fn check_graph_kind(node: &Node, kind: GraphKind) -> Result<(), WorkflowError> {
    match node.kind() {
        NodeKind::Switch { cases, .. } => {
            let invalid = |reason| WorkflowError::InvalidSwitch {
                node: node.id().clone(),
                reason,
            };
            if cases.is_empty() || cases.len() > MAX_SWITCH_CASES {
                return Err(invalid("case count must be in 1..=32"));
            }
            if cases
                .iter()
                .map(SwitchCase::id)
                .collect::<BTreeSet<_>>()
                .len()
                != cases.len()
            {
                return Err(invalid("case ids must be unique"));
            }
            if node.next().is_some() || node.on_error().is_some() || node.timeout().is_some() {
                return Err(invalid("switch cannot declare next, onError or timeout"));
            }
            if cases.iter().any(|case| case.when().as_str().len() > 1024) {
                return Err(invalid("predicate exceeds the supported byte limit"));
            }
        }
        NodeKind::Approval { .. } => {
            if kind == GraphKind::Legacy {
                return Err(scope_error(
                    node.id(),
                    "approval requires a structured definition",
                ));
            }
            if node.timeout().is_none() || node.next().is_some() {
                return Err(scope_error(
                    node.id(),
                    "approval requires a timeout and outcome routes",
                ));
            }
        }
        NodeKind::ForEach {
            max_items,
            max_concurrency,
            ..
        } => {
            if kind == GraphKind::Legacy {
                return Err(scope_error(
                    node.id(),
                    "collection requires a structured definition",
                ));
            }
            if !(1..=10_000).contains(max_items) || !(1..=256).contains(max_concurrency) {
                return Err(scope_error(
                    node.id(),
                    "collection item/concurrency limit is out of range",
                ));
            }
            if node.next().is_none() {
                return Err(scope_error(node.id(), "collection requires a successor"));
            }
        }
        NodeKind::While {
            max_iterations,
            delay,
            ..
        }
        | NodeKind::RepeatUntil {
            max_iterations,
            delay,
            ..
        } => {
            if kind == GraphKind::Legacy {
                return Err(scope_error(
                    node.id(),
                    "loop requires a structured definition",
                ));
            }
            if !(1..=10_000).contains(max_iterations) {
                return Err(scope_error(
                    node.id(),
                    "loop iteration limit is out of range",
                ));
            }
            if node.timeout().is_none() {
                return Err(scope_error(node.id(), "loop requires a timeout"));
            }
            if node.next().is_none() {
                return Err(scope_error(node.id(), "loop requires a successor"));
            }
            if let Some(delay) = delay {
                duration_in_range(node.id(), "loop delay", *delay)?;
            }
        }
        NodeKind::Return { .. } => {
            if kind != GraphKind::Body {
                return Err(scope_error(
                    node.id(),
                    "return is only legal in a named body",
                ));
            }
            if node.next().is_some() || node.on_error().is_some() || node.timeout().is_some() {
                return Err(scope_error(
                    node.id(),
                    "return cannot route or own an attempt timeout",
                ));
            }
        }
        NodeKind::End { .. } if kind == GraphKind::Body => {
            return Err(scope_error(
                node.id(),
                "workflow end is forbidden inside a body",
            ));
        }
        _ => {}
    }
    if kind != GraphKind::Legacy {
        let expressions: Vec<&Expression> = match node.kind() {
            NodeKind::Activity {
                idempotency_template,
                ..
            } => idempotency_template.iter().collect(),
            NodeKind::WaitEvent { correlation, .. } => vec![correlation],
            NodeKind::Condition { expression, .. } => vec![expression],
            NodeKind::Switch { cases, .. } => cases.iter().map(SwitchCase::when).collect(),
            NodeKind::Child { input, .. } => input.iter().collect(),
            NodeKind::End { output } => output.iter().collect(),
            NodeKind::ForEach { items, result, .. } => vec![items, result],
            NodeKind::While {
                condition,
                initial,
                carry,
                ..
            }
            | NodeKind::RepeatUntil {
                condition,
                initial,
                carry,
                ..
            } => vec![condition, initial, carry],
            NodeKind::Return { result } => vec![result],
            _ => Vec::new(),
        };
        if expressions
            .iter()
            .any(|expression| expression.as_str().len() > 1024)
        {
            return Err(scope_error(
                node.id(),
                "expression exceeds the supported byte limit",
            ));
        }
    }
    Ok(())
}

fn check_body_return(
    nodes: &BTreeMap<NodeId, Node>,
    entrypoint: &NodeId,
    scopes: &BTreeMap<NodeId, Scope>,
) -> Result<(), WorkflowError> {
    if nodes
        .values()
        .filter(|node| matches!(node.kind(), NodeKind::Return { .. }))
        .count()
        != 1
    {
        return Err(scope_error(entrypoint, "a body needs exactly one return"));
    }
    for node in nodes.values() {
        let explicit_route = matches!(
            node.kind(),
            NodeKind::Condition { .. } | NodeKind::Return { .. }
        ) || node.next().is_some();
        if !explicit_route && !scopes.contains_key(node.id()) {
            return Err(scope_error(
                node.id(),
                "a successful body path cannot end before return",
            ));
        }
    }
    Ok(())
}

fn body_dependencies(
    graph: &DefinitionGraph,
    bodies: &BTreeMap<NodeId, BodyDefinition>,
) -> Result<BTreeSet<NodeId>, WorkflowError> {
    let mut dependencies = BTreeSet::new();
    for node in graph.nodes.values() {
        let body = match node.kind() {
            NodeKind::ForEach { body, .. }
            | NodeKind::While { body, .. }
            | NodeKind::RepeatUntil { body, .. } => Some(body),
            _ => None,
        };
        let Some(body) = body else { continue };
        if !bodies.contains_key(body) {
            return Err(WorkflowError::UnknownBody {
                node: node.id.clone(),
                body: body.clone(),
            });
        }
        dependencies.insert(body.clone());
    }
    Ok(dependencies)
}

fn check_body_references(
    root: &DefinitionGraph,
    bodies: &BTreeMap<NodeId, BodyDefinition>,
    limits: WorkflowLimits,
) -> Result<(), WorkflowError> {
    let root_dependencies = body_dependencies(root, bodies)?;
    let mut dependencies = BTreeMap::new();
    let mut dependants: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    for (name, body) in bodies {
        let children = body_dependencies(&body.graph, bodies)?;
        for child in &children {
            dependants
                .entry(child.clone())
                .or_default()
                .push(name.clone());
        }
        dependencies.insert(name.clone(), children);
    }
    let mut reached = BTreeSet::new();
    let mut pending: Vec<_> = root_dependencies.iter().cloned().collect();
    while let Some(name) = pending.pop() {
        if reached.insert(name.clone()) {
            pending.extend(dependencies[&name].iter().cloned());
        }
    }
    if let Some(unreachable) = bodies.keys().find(|name| !reached.contains(*name)) {
        return Err(WorkflowError::UnreachableBody(unreachable.clone()));
    }
    // Resolve leaves before owners. Each reference is visited once, even when many scopes share a body.
    let mut remaining: BTreeMap<_, _> = dependencies
        .iter()
        .map(|(name, children)| (name.clone(), children.len()))
        .collect();
    let mut ready: Vec<_> = remaining
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(name, _)| name.clone())
        .collect();
    let mut depths: BTreeMap<NodeId, usize> = bodies.keys().map(|name| (name.clone(), 1)).collect();
    let mut completed = 0;
    while let Some(name) = ready.pop() {
        completed += 1;
        let next_depth = depths[&name] + 1;
        for parent in dependants.get(&name).into_iter().flatten() {
            if let Some(depth) = depths.get_mut(parent) {
                *depth = (*depth).max(next_depth);
            }
            if let Some(count) = remaining.get_mut(parent) {
                *count -= 1;
                if *count == 0 {
                    ready.push(parent.clone());
                }
            }
        }
    }
    if completed != bodies.len()
        && let Some((name, _)) = remaining.iter().find(|(_, count)| **count != 0)
    {
        return Err(WorkflowError::RecursiveBody(name.clone()));
    }
    if root_dependencies
        .iter()
        .any(|name| depths[name] > usize::from(limits.max_depth))
    {
        return Err(WorkflowError::BodyDepthExceeded(limits.max_depth));
    }
    Ok(())
}

fn check_node(node: &Node) -> Result<(), WorkflowError> {
    if let Some(timeout) = node.timeout {
        duration_in_range(&node.id, "timeout", timeout)?;
    }
    if node.effect != EffectPolicy::NONE {
        match &node.kind {
            NodeKind::Activity {
                compensate_with, ..
            } => {
                if node.effect.irreversible && compensate_with.is_some() {
                    return Err(WorkflowError::IrreversibleWithCompensation(node.id.clone()));
                }
                if let Some(timeout) = node.effect.compensation_timeout {
                    duration_in_range(&node.id, "compensation timeout", timeout)?;
                }
            }
            _ => return Err(WorkflowError::EffectOnNonActivity(node.id.clone())),
        }
    }
    match &node.kind {
        NodeKind::Timer { delay } => duration_in_range(&node.id, "delay", *delay)?,
        NodeKind::WaitEvent {
            timeout, lookback, ..
        } => {
            duration_in_range(&node.id, "wait timeout", *timeout)?;
            if *lookback > MAX_LOOKBACK {
                return Err(WorkflowError::DurationOutOfRange {
                    node: node.id.clone(),
                    what: "lookback",
                    millis: lookback.as_millis(),
                    max_millis: MAX_LOOKBACK.as_millis(),
                });
            }
        }
        NodeKind::Parallel { branches, .. } => {
            let distinct: BTreeSet<&NodeId> = branches.iter().collect();
            if branches.len() < 2
                || branches.len() > MAX_BRANCHES
                || distinct.len() != branches.len()
            {
                return Err(WorkflowError::InvalidBranchCount(node.id.clone()));
            }
        }
        _ => {}
    }
    Ok(())
}

/// Iterative three-colour DFS (the graph can be 10 000 nodes deep, so no recursion).
fn check_acyclic(map: &BTreeMap<NodeId, Node>) -> Result<(), WorkflowError> {
    #[derive(Clone, Copy, PartialEq)]
    enum Colour {
        Grey,
        Black,
    }
    let mut colour: BTreeMap<&NodeId, Colour> = BTreeMap::new();
    for root in map.keys() {
        if colour.contains_key(root) {
            continue;
        }
        let mut stack: Vec<(&NodeId, Vec<&NodeId>, usize)> = Vec::new();
        colour.insert(root, Colour::Grey);
        stack.push((root, map[root].successors(), 0));
        while let Some((id, succ, idx)) = stack.last_mut() {
            if let Some(&target) = succ.get(*idx) {
                *idx += 1;
                match colour.get(target) {
                    Some(Colour::Grey) => return Err(WorkflowError::Cycle(target.clone())),
                    Some(Colour::Black) => {}
                    None => {
                        colour.insert(target, Colour::Grey);
                        stack.push((target, map[target].successors(), 0));
                    }
                }
            } else {
                colour.insert(id, Colour::Black);
                stack.pop();
            }
        }
    }
    Ok(())
}

fn reach<'a>(map: &'a BTreeMap<NodeId, Node>, start: &'a NodeId) -> BTreeSet<&'a NodeId> {
    let mut seen: BTreeSet<&NodeId> = BTreeSet::new();
    let mut queue = vec![start];
    while let Some(id) = queue.pop() {
        if !seen.insert(id) {
            continue;
        }
        if let Some(node) = map.get(id) {
            queue.extend(node.successors());
        }
    }
    seen
}

fn check_reachable(map: &BTreeMap<NodeId, Node>, entrypoint: &NodeId) -> Result<(), WorkflowError> {
    let seen = reach(map, entrypoint);
    map.keys()
        .find(|id| !seen.contains(id))
        .map_or(Ok(()), |id| Err(WorkflowError::Unreachable(id.clone())))
}

type Regions = BTreeMap<NodeId, Vec<BTreeSet<NodeId>>>;

fn parallel_regions(map: &BTreeMap<NodeId, Node>) -> Result<Regions, WorkflowError> {
    let mut preds: BTreeMap<&NodeId, BTreeSet<&NodeId>> = BTreeMap::new();
    for node in map.values() {
        for target in node.successors() {
            preds.entry(target).or_default().insert(node.id());
        }
    }
    let mut out = Regions::new();
    for node in map.values() {
        let NodeKind::Parallel { branches, .. } = node.kind() else {
            continue;
        };
        let mut regions: Vec<BTreeSet<&NodeId>> = Vec::with_capacity(branches.len());
        for start in branches {
            regions.push(reach(map, start));
        }
        for (i, region) in regions.iter().enumerate() {
            for (j, other) in regions.iter().enumerate().skip(i + 1) {
                if let Some(shared) = region.intersection(other).next() {
                    return Err(WorkflowError::BranchOverlap {
                        parallel: node.id().clone(),
                        a: branches[i].clone(),
                        b: branches[j].clone(),
                        node: (*shared).clone(),
                    });
                }
            }
            for &member in region {
                if matches!(
                    map[member].kind(),
                    NodeKind::End { .. } | NodeKind::Return { .. }
                ) {
                    return Err(WorkflowError::EndInsideBranch {
                        parallel: node.id().clone(),
                        node: member.clone(),
                    });
                }
                let entered_only_from_inside = preds.get(member).is_none_or(|p| {
                    p.iter().all(|&q| {
                        region.contains(q) || (q == node.id() && branches.contains(member))
                    })
                });
                if !entered_only_from_inside {
                    return Err(WorkflowError::BranchBoundary {
                        parallel: node.id().clone(),
                        node: member.clone(),
                    });
                }
            }
        }
        out.insert(
            node.id().clone(),
            regions
                .into_iter()
                .map(|r| r.into_iter().cloned().collect())
                .collect(),
        );
    }
    Ok(out)
}

/// The node set of the body of every `saga` node, checked like the branches of a `parallel` node:
/// the body is entered only from inside, and holds no workflow `end`.
fn saga_regions(
    map: &BTreeMap<NodeId, Node>,
) -> Result<BTreeMap<NodeId, BTreeSet<NodeId>>, WorkflowError> {
    let mut preds: BTreeMap<&NodeId, BTreeSet<&NodeId>> = BTreeMap::new();
    for node in map.values() {
        for target in node.successors() {
            preds.entry(target).or_default().insert(node.id());
        }
    }
    let mut out = BTreeMap::new();
    for node in map.values() {
        let NodeKind::Saga { begin, .. } = node.kind() else {
            continue;
        };
        let region = reach(map, begin);
        for &member in &region {
            if matches!(
                map[member].kind(),
                NodeKind::End { .. } | NodeKind::Return { .. }
            ) {
                return Err(WorkflowError::EndInsideSaga {
                    saga: node.id().clone(),
                    node: member.clone(),
                });
            }
            let entered_only_from_inside = preds.get(member).is_none_or(|p| {
                p.iter()
                    .all(|&q| region.contains(q) || (q == node.id() && member == begin))
            });
            if !entered_only_from_inside {
                return Err(WorkflowError::SagaBoundary {
                    saga: node.id().clone(),
                    node: member.clone(),
                });
            }
        }
        out.insert(
            node.id().clone(),
            region.into_iter().cloned().collect::<BTreeSet<_>>(),
        );
    }
    Ok(out)
}

/// Maps every node inside some region to the owner of the smallest region around it. Regions nest
/// strictly (each is entered only from inside, and excludes its owner), so the smallest is unique.
fn innermost_scopes(
    regions: &Regions,
    sagas: &BTreeMap<NodeId, BTreeSet<NodeId>>,
) -> BTreeMap<NodeId, Scope> {
    let mut found: BTreeMap<NodeId, (usize, Scope)> = BTreeMap::new();
    for (parallel, branches) in regions {
        let size: usize = branches.iter().map(BTreeSet::len).sum();
        for node in branches.iter().flatten() {
            let better = found.get(node).is_none_or(|(current, _)| size < *current);
            if better {
                found.insert(node.clone(), (size, Scope::Parallel(parallel.clone())));
            }
        }
    }
    for (saga, region) in sagas {
        let size = region.len();
        for node in region {
            let better = found.get(node).is_none_or(|(current, _)| size < *current);
            if better {
                found.insert(node.clone(), (size, Scope::Saga(saga.clone())));
            }
        }
    }
    found
        .into_iter()
        .map(|(node, (_, scope))| (node, scope))
        .collect()
}

/// The compensation and the reconciliation nodes that `compensateWith` and `reconcileWith` stand for.
fn derive_companions(map: &BTreeMap<NodeId, Node>) -> Vec<Node> {
    let mut out = Vec::new();
    for node in map.values() {
        let NodeKind::Activity {
            compensate_with,
            reconcile_with,
            secrets,
            ..
        } = node.kind()
        else {
            continue;
        };
        // The same capabilities as the forward action: undoing a payment needs the credentials that
        // made it.
        let derive = |id: NodeId, activity: &ActivityName, retry: RetryPolicy, origin| {
            let mut derived = Node::new(
                id,
                NodeKind::Activity {
                    activity: activity.clone(),
                    retry,
                    compensate_with: None,
                    reconcile_with: None,
                    idempotency_template: None,
                    secrets: secrets.clone(),
                },
            );
            derived.origin = origin;
            derived
        };
        if let Some(activity) = compensate_with {
            let retry = node
                .effect
                .compensation_retry
                .unwrap_or(RetryPolicy::COMPENSATION);
            let mut derived = derive(
                node.id.compensation(),
                activity,
                retry,
                NodeOrigin::Compensation(node.id.clone()),
            );
            derived.timeout = node.effect.compensation_timeout;
            out.push(derived);
        }
        if let Some(activity) = reconcile_with {
            out.push(derive(
                node.id.reconciliation(),
                activity,
                RetryPolicy::RECONCILIATION,
                NodeOrigin::Reconciliation(node.id.clone()),
            ));
        }
    }
    out
}
