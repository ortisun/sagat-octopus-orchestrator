//! Workflow definitions: the typed form of the IR (`workflow.schema.json`).
//!
//! A [`WorkflowDefinition`] can only be built through [`WorkflowDefinition::new`], which proves the
//! properties the engine relies on instead of re-checking them at run time:
//!
//! * every node id is unique and every reference points at an existing node;
//! * the graph is acyclic (loops are a separate, bounded construct that is not part of this
//!   version) and every node is reachable from the entrypoint;
//! * the branches of a `parallel` node are single-entry regions that do not overlap, do not leave
//!   the region and do not contain a workflow `end`, so a join is well defined;
//! * delays, timeouts, retry budgets and fan-out are bounded.
//!
//! Wire formats (JSON) are an adapter concern: this module holds no serialization, and expressions
//! are opaque text evaluated by an adapter (`FR-002-001`, ADR-0003).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::Duration;

use crate::identity::{Slug, clip};

/// A workflow may have at most this many nodes (matches `nodes.maxItems` in the schema).
pub const MAX_NODES: usize = 10_000;
/// A `parallel` node may fan out to at most this many branches.
pub const MAX_BRANCHES: usize = 256;
/// Longest timer or wait: a run waiting for months is viable, one waiting for decades is a mistake.
pub const MAX_WAIT: Duration = Duration::from_secs(366 * 24 * 60 * 60);
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
    /// second up to one minute, with jitter. A decision, recorded in `research.md` (D18).
    pub const DEFAULT: Self = Self {
        max_attempts: 3,
        initial_delay: Duration::from_secs(1),
        max_delay: Duration::from_secs(60),
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

/// What a node does (the node kinds of version 1 that this engine executes).
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
    },
    /// Waits for a delay without holding any process.
    Timer { delay: Duration },
    /// Waits for a correlated event, with a deadline.
    WaitEvent {
        event_type: EventType,
        correlation: Expression,
        timeout: Duration,
        on_timeout: Option<NodeId>,
    },
    /// Chooses between two successors by a side-effect-free expression.
    Condition {
        expression: Expression,
        if_true: NodeId,
        if_false: NodeId,
    },
    /// Fans out to `branches` and joins them according to `join`.
    Parallel {
        branches: Vec<NodeId>,
        join: JoinPolicy,
    },
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
            Self::Parallel { .. } => "parallel",
            Self::End { .. } => "end",
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
        }
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

    /// The retry policy that governs this node (only activities retry).
    #[must_use]
    pub const fn retry_policy(&self) -> RetryPolicy {
        match &self.kind {
            NodeKind::Activity { retry, .. } => *retry,
            _ => RetryPolicy::no_retry(),
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
            NodeKind::Parallel { branches, .. } => out.extend(branches.iter()),
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
    /// `compensateWith` / `reconcileWith` are stored but not executed until the saga engine exists.
    SagaFieldsNotExecuted { node: NodeId },
}

impl fmt::Display for DefinitionWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SagaFieldsNotExecuted { node } => write!(
                f,
                "node '{node}': compensateWith/reconcileWith are recorded but compensation is not executed yet (feature 003)"
            ),
        }
    }
}

/// A validated, immutable workflow graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowDefinition {
    name: Slug,
    entrypoint: NodeId,
    nodes: BTreeMap<NodeId, Node>,
    /// For every `parallel` node, the node set of each branch (index-aligned with `branches`).
    regions: BTreeMap<NodeId, Vec<BTreeSet<NodeId>>>,
    /// The innermost `parallel` node whose branches contain a node.
    enclosing: BTreeMap<NodeId, NodeId>,
}

impl WorkflowDefinition {
    /// Validates `nodes` and builds the definition.
    ///
    /// # Errors
    /// The first violated rule, as a [`WorkflowError`].
    pub fn new(name: Slug, entrypoint: NodeId, nodes: Vec<Node>) -> Result<Self, WorkflowError> {
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
            for target in node.successors() {
                if !map.contains_key(target) {
                    return Err(WorkflowError::UnknownTarget {
                        from: node.id.clone(),
                        to: target.clone(),
                    });
                }
            }
        }
        check_acyclic(&map)?;
        check_reachable(&map, &entrypoint)?;
        let regions = parallel_regions(&map)?;
        let enclosing = innermost_parallels(&regions);
        Ok(Self {
            name,
            entrypoint,
            nodes: map,
            regions,
            enclosing,
        })
    }

    #[must_use]
    pub const fn name(&self) -> &Slug {
        &self.name
    }

    #[must_use]
    pub const fn entrypoint(&self) -> &NodeId {
        &self.entrypoint
    }

    #[must_use]
    pub fn node(&self, id: &NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// All nodes, ordered by id.
    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The node sets of the branches of a `parallel` node, aligned with its `branches`.
    #[must_use]
    pub fn branch_regions(&self, parallel: &NodeId) -> Option<&[BTreeSet<NodeId>]> {
        self.regions.get(parallel).map(Vec::as_slice)
    }

    /// The innermost `parallel` node whose branches contain `node`, if any.
    #[must_use]
    pub fn enclosing_parallel(&self, node: &NodeId) -> Option<&NodeId> {
        self.enclosing.get(node)
    }

    /// Observations for the author; none of them makes the definition invalid.
    #[must_use]
    pub fn warnings(&self) -> Vec<DefinitionWarning> {
        self.nodes()
            .filter(|n| {
                matches!(
                    n.kind(),
                    NodeKind::Activity {
                        compensate_with: Some(_),
                        ..
                    } | NodeKind::Activity {
                        reconcile_with: Some(_),
                        ..
                    }
                )
            })
            .map(|n| DefinitionWarning::SagaFieldsNotExecuted {
                node: n.id().clone(),
            })
            .collect()
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

fn check_node(node: &Node) -> Result<(), WorkflowError> {
    if let Some(timeout) = node.timeout {
        duration_in_range(&node.id, "timeout", timeout)?;
    }
    match &node.kind {
        NodeKind::Timer { delay } => duration_in_range(&node.id, "delay", *delay)?,
        NodeKind::WaitEvent { timeout, .. } => {
            duration_in_range(&node.id, "wait timeout", *timeout)?
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
                if matches!(map[member].kind(), NodeKind::End { .. }) {
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

/// Maps every node inside some branch to the parallel node with the smallest region around it.
fn innermost_parallels(regions: &Regions) -> BTreeMap<NodeId, NodeId> {
    let mut best: BTreeMap<&NodeId, (usize, &NodeId)> = BTreeMap::new();
    for (parallel, branches) in regions {
        let size: usize = branches.iter().map(BTreeSet::len).sum();
        for node in branches.iter().flatten() {
            let better = best.get(node).is_none_or(|(current, _)| size < *current);
            if better {
                best.insert(node, (size, parallel));
            }
        }
    }
    best.into_iter()
        .map(|(node, (_, parallel))| (node.clone(), parallel.clone()))
        .collect()
}
