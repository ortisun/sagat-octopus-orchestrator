//! Flow control of a definition (feature 022, release 0.3.0 slice: US02 concurrency keys, US07 rate limits).
//!
//! A definition may declare, at the top level (run starts) and on a root `activity` or `script` node
//! (attempt dispatches), a [`ConcurrencyPolicy`] (at most `limit` holders of one key at once, ADR-0056 and
//! ADR-0089) and a [`RateLimitPolicy`] (at most `limit` grants of one key in any window of `per`, ADR-0088).
//! This module holds the typed, bounded declarations and their pure validation; how a run waits, is granted
//! or times out is the journal's business (`execution::flow_control`), and the key value is never seen here:
//! a key is an expression, evaluated and digested by an adapter (D1168).
//!
//! Bounds are the hard ceilings of the plan (`specs/022-flow-control-step-policies/plan.md`, "Finite ceilings"
//! and "Ceilings, revised"). A namespace policy or the deployment may lower what an acquirer gets at admission
//! (FR-022-047); publication refuses only what no policy could ever allow.

use std::fmt;
use std::time::Duration;

use super::{Expression, NodeId, WorkflowError};
use crate::identity::{Slug, clip};

/// The most holders one key of a concurrency group may have (plan, "Finite ceilings").
pub const MAX_CONCURRENCY_LIMIT: u32 = 10_000;
/// The queue timeout when a declaration names none: ten minutes.
pub const DEFAULT_QUEUE_TIMEOUT: Duration = Duration::from_millis(600_000);
/// The longest a run or a step may wait for a permit or a slot: one day.
pub const MAX_QUEUE_TIMEOUT: Duration = Duration::from_millis(86_400_000);
/// The longest canonical value a concurrency or rate-limit key may have, in bytes (FR-022-054).
pub const MAX_KEY_BYTES: usize = 256;
/// Node-level declarations of one kind a definition may hold by default; a namespace policy may raise it to
/// [`MAX_NODE_DECLARATIONS`] (plan, "Finite ceilings": 1 run + 16 node by default, 1 run + 64 node at most).
pub const DEFAULT_NODE_DECLARATIONS: usize = 16;
/// Node-level declarations of one kind a definition may hold whatever the policy.
pub const MAX_NODE_DECLARATIONS: usize = 64;

const MAX_GROUP_LEN: usize = 64;
/// `=` + a workflow slug (at most 64) + `/` + a node id (at most 64).
const MAX_DERIVED_GROUP_LEN: usize = 130;

/// An explicit group name: `^[a-z][a-z0-9.-]{0,63}$`. Definitions of one namespace that name the same group at
/// the same level share its keys; a concurrency group and a rate-limit group of the same name are unrelated.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GroupName(String);

impl GroupName {
    /// # Errors
    /// [`FlowControlRule::Group`] when the value breaks the pattern.
    pub fn parse(value: &str) -> Result<Self, FlowControlRule> {
        let mut chars = value.chars();
        let valid = value.len() <= MAX_GROUP_LEN
            && chars.next().is_some_and(|c| c.is_ascii_lowercase())
            && chars
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-'));
        if valid {
            Ok(Self(value.to_owned()))
        } else {
            Err(FlowControlRule::Group)
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The group of a declaration: the one it names, or the default derived from where it stands.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FlowGroup {
    Explicit(GroupName),
    /// `=<workflow>` for a run-level declaration, `=<workflow>/<node>` for a node-level one. The leading `=`
    /// is outside the pattern of an explicit name, so no author can name a derived group.
    Derived,
}

impl FlowGroup {
    /// The name a key row is filed under. Never longer than 130 characters.
    #[must_use]
    pub fn resolve(&self, workflow: &Slug, node: Option<&NodeId>) -> ResolvedGroup {
        match self {
            Self::Explicit(name) => ResolvedGroup(name.as_str().to_owned()),
            Self::Derived => ResolvedGroup(match node {
                None => format!("={workflow}"),
                Some(node) => format!("={workflow}/{node}"),
            }),
        }
    }
}

/// A group as the rows and the journal store it: an explicit name or a derived `=…` one.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResolvedGroup(String);

impl ResolvedGroup {
    /// Reads a stored group back (a journal row, a key row).
    ///
    /// # Errors
    /// [`FlowControlRule::Group`] for a value that neither an explicit nor a derived group can have.
    pub fn parse(value: &str) -> Result<Self, FlowControlRule> {
        if let Some(derived) = value.strip_prefix('=') {
            let (workflow, node) = match derived.split_once('/') {
                Some((workflow, node)) => (workflow, Some(node)),
                None => (derived, None),
            };
            let valid = value.len() <= MAX_DERIVED_GROUP_LEN
                && Slug::parse(workflow).is_ok()
                && node.is_none_or(|node| NodeId::parse(node).is_ok());
            if valid {
                return Ok(Self(value.to_owned()));
            }
            return Err(FlowControlRule::Group);
        }
        GroupName::parse(value).map(|name| Self(name.0))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether the group was derived from where the declaration stands.
    #[must_use]
    pub fn is_derived(&self) -> bool {
        self.0.starts_with('=')
    }
}

impl fmt::Display for ResolvedGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where a declaration stands: on the definition (it limits run starts) or on a node (it limits the attempts of
/// that step). A run-level and a step-level declaration never share a key row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GateLevel {
    Run,
    Step,
}

impl GateLevel {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Run => "RUN",
            Self::Step => "STEP",
        }
    }
}

/// The window of a rate limit: a closed set (D022-002), so the retained grants and the estimate have a bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RatePeriod {
    Second,
    Minute,
    Hour,
}

impl RatePeriod {
    pub const ALL: [Self; 3] = [Self::Second, Self::Minute, Self::Hour];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Second => "second",
            Self::Minute => "minute",
            Self::Hour => "hour",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|period| period.as_str() == value)
    }

    /// The length of the window.
    #[must_use]
    pub const fn duration(self) -> Duration {
        Duration::from_millis(self.millis())
    }

    /// The length of the window in milliseconds, as `rate_limit_grants.period_ms` stores it.
    #[must_use]
    pub const fn millis(self) -> u64 {
        match self {
            Self::Second => 1_000,
            Self::Minute => 60_000,
            Self::Hour => 3_600_000,
        }
    }

    /// Reads a stored period back.
    #[must_use]
    pub const fn from_millis(millis: u64) -> Option<Self> {
        match millis {
            1_000 => Some(Self::Second),
            60_000 => Some(Self::Minute),
            3_600_000 => Some(Self::Hour),
            _ => None,
        }
    }

    /// The most grants one key may count in a window of this length, whatever the policy (plan, "Finite
    /// ceilings (rate limits)"). The deployment may publish a lower ceiling (FR-022-047, SC-022-029).
    #[must_use]
    pub const fn max_limit(self) -> u32 {
        match self {
            Self::Second => 100,
            Self::Minute => 6_000,
            Self::Hour => 10_000,
        }
    }
}

/// The rule of a flow-control declaration that a value broke. Messages name the rule, never a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FlowControlRule {
    #[error("the limit must be at least 1 and at most {max}")]
    Limit { max: u32 },
    #[error("the group must match ^[a-z][a-z0-9.-]{{0,63}}$")]
    Group,
    #[error("the queue timeout must be between 1 ms and 86400000 ms")]
    QueueTimeout,
    #[error("the window must be one of second, minute or hour")]
    Period,
    #[error("only a root activity or script node may declare it")]
    NodeKind,
    #[error("a top-level key reads only `input`")]
    TopLevelKey,
    #[error("a definition may hold at most {max} node-level declarations of one kind")]
    TooManyDeclarations { max: usize },
}

/// At most `limit` holders of one key at once (FR-022-017). `key` is absent for the single key of the group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConcurrencyPolicy {
    group: FlowGroup,
    key: Option<Expression>,
    limit: u32,
    queue_timeout: Duration,
}

impl ConcurrencyPolicy {
    /// # Errors
    /// [`FlowControlRule::Limit`] or [`FlowControlRule::QueueTimeout`] for a value outside its bound.
    pub fn new(
        group: FlowGroup,
        key: Option<Expression>,
        limit: u32,
        queue_timeout: Duration,
    ) -> Result<Self, FlowControlRule> {
        if !(1..=MAX_CONCURRENCY_LIMIT).contains(&limit) {
            return Err(FlowControlRule::Limit {
                max: MAX_CONCURRENCY_LIMIT,
            });
        }
        check_queue_timeout(queue_timeout)?;
        Ok(Self {
            group,
            key,
            limit,
            queue_timeout,
        })
    }

    #[must_use]
    pub const fn group(&self) -> &FlowGroup {
        &self.group
    }

    #[must_use]
    pub const fn key(&self) -> Option<&Expression> {
        self.key.as_ref()
    }

    #[must_use]
    pub const fn limit(&self) -> u32 {
        self.limit
    }

    #[must_use]
    pub const fn queue_timeout(&self) -> Duration {
        self.queue_timeout
    }
}

/// At most `limit` grants of one key in any window of `per` on the database clock (FR-022-039, FR-022-040).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitPolicy {
    group: FlowGroup,
    key: Option<Expression>,
    limit: u32,
    per: RatePeriod,
    queue_timeout: Duration,
}

impl RateLimitPolicy {
    /// # Errors
    /// [`FlowControlRule::Limit`] when `limit` is zero or above the ceiling of `per`;
    /// [`FlowControlRule::QueueTimeout`] for a timeout outside its bound.
    pub fn new(
        group: FlowGroup,
        key: Option<Expression>,
        limit: u32,
        per: RatePeriod,
        queue_timeout: Duration,
    ) -> Result<Self, FlowControlRule> {
        if !(1..=per.max_limit()).contains(&limit) {
            return Err(FlowControlRule::Limit {
                max: per.max_limit(),
            });
        }
        check_queue_timeout(queue_timeout)?;
        Ok(Self {
            group,
            key,
            limit,
            per,
            queue_timeout,
        })
    }

    #[must_use]
    pub const fn group(&self) -> &FlowGroup {
        &self.group
    }

    #[must_use]
    pub const fn key(&self) -> Option<&Expression> {
        self.key.as_ref()
    }

    #[must_use]
    pub const fn limit(&self) -> u32 {
        self.limit
    }

    #[must_use]
    pub const fn per(&self) -> RatePeriod {
        self.per
    }

    #[must_use]
    pub const fn queue_timeout(&self) -> Duration {
        self.queue_timeout
    }
}

fn check_queue_timeout(timeout: Duration) -> Result<(), FlowControlRule> {
    if timeout.is_zero()
        || timeout > MAX_QUEUE_TIMEOUT
        || !timeout.subsec_nanos().is_multiple_of(1_000_000)
    {
        return Err(FlowControlRule::QueueTimeout);
    }
    Ok(())
}

/// The gates one level declares: either, both or none. A run (or a step) that declares both is given the
/// permit and the slot together or neither (FR-022-044).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowControl {
    concurrency: Option<ConcurrencyPolicy>,
    rate_limit: Option<RateLimitPolicy>,
}

impl FlowControl {
    pub const NONE: Self = Self {
        concurrency: None,
        rate_limit: None,
    };

    #[must_use]
    pub const fn new(
        concurrency: Option<ConcurrencyPolicy>,
        rate_limit: Option<RateLimitPolicy>,
    ) -> Self {
        Self {
            concurrency,
            rate_limit,
        }
    }

    #[must_use]
    pub const fn concurrency(&self) -> Option<&ConcurrencyPolicy> {
        self.concurrency.as_ref()
    }

    #[must_use]
    pub const fn rate_limit(&self) -> Option<&RateLimitPolicy> {
        self.rate_limit.as_ref()
    }

    /// Whether this level declares any gate.
    #[must_use]
    pub const fn is_gated(&self) -> bool {
        self.concurrency.is_some() || self.rate_limit.is_some()
    }

    /// The single wait of a level that declares both gates: the earlier of the two timeouts (FR-022-044).
    #[must_use]
    pub fn queue_timeout(&self) -> Option<Duration> {
        match (&self.concurrency, &self.rate_limit) {
            (Some(c), Some(r)) => Some(c.queue_timeout.min(r.queue_timeout)),
            (Some(c), None) => Some(c.queue_timeout),
            (None, Some(r)) => Some(r.queue_timeout),
            (None, None) => None,
        }
    }
}

/// Which kind of gate a refused declaration is, for the problem code it is reported under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateKind {
    /// `CONCURRENCY_POLICY_INVALID`.
    Concurrency,
    /// `RATE_LIMIT_POLICY_INVALID`.
    RateLimit,
}

impl GateKind {
    /// The field of the IR that declares it.
    #[must_use]
    pub const fn field(self) -> &'static str {
        match self {
            Self::Concurrency => "concurrency",
            Self::RateLimit => "rateLimit",
        }
    }
}

impl fmt::Display for GateKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.field())
    }
}

/// The error of a refused declaration, naming where it stands. `node` is `None` for a top-level declaration.
#[must_use]
pub fn invalid(kind: GateKind, node: Option<&NodeId>, rule: FlowControlRule) -> WorkflowError {
    WorkflowError::InvalidFlowControl {
        kind,
        node: node.map(|node| clip(node.as_str())),
        rule,
    }
}
