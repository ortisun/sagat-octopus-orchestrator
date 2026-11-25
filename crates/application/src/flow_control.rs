//! Flow control of feature 022 in the application layer (release 0.3.0 slice): which capabilities a definition needs,
//! the deployment switches that allow them, their fleet activation, and the typed refusals of `contracts/errors.md`.
//!
//! Two switches, enabled one at a time (FR-022-031, FR-022-050): `concurrencyKeys` for `concurrency`, `rateLimits` for
//! `rateLimit`. Each is also an instance capability of feature 018's registry (FR-022-053, ADR-0089 decision 6): until an
//! operator activates it for the fleet, publication and starts that use it are refused
//! `FLOW_CONTROL_ACTIVATION_PENDING`. A switch that is off refuses before the activation is even read.
//!
//! What a build does not have is said by the same types, not hidden: a process lists a capability only when it runs
//! the gate (D022-037), and the one admission point refuses a gated definition the process does not serve with
//! [`FlowControlRefusal::BinaryCapabilityMissing`] (FR-022-059). This build runs both gates, for a process whose switch
//! is on and which holds the deployment's digest keys ([`served`], D022-057).

use std::collections::BTreeSet;

use sagat_domain::workflow::{GateKind, WorkflowDefinition};

use crate::instances::Capability;

/// The flow-control capabilities this build runs the gate of (D022-057): the concurrency gate (admission, readiness,
/// grant, timeout, release) and the rate gate (admission and readiness, grant, timeout, return, the lease instant of a
/// granted step and the purge of aged grants; T022-072, T022-074), with the namespace ceilings of both (G1199, T022-077).
pub const BUILT: [FlowControlCapability; 2] = [
    FlowControlCapability::ConcurrencyKeys,
    FlowControlCapability::RateLimits,
];

/// The flow-control capabilities a process serves: built, switched on, and backed by the deployment's digest keys
/// (`keys`). What its admission point admits (D022-057).
#[must_use]
pub fn served(switches: FlowControlSwitches, keys: bool) -> Vec<FlowControlCapability> {
    BUILT
        .into_iter()
        .filter(|capability| keys && switches.enabled(*capability))
        .collect()
}

/// The flow-control capabilities a process lists in the instance registry: built and backed by the digest keys, whatever
/// its switches (D022-062 as amended). A listed capability promises that the process runs the gate (decides, grants,
/// times out what was admitted); the switch says only whether it admits new gated work. So a process switched off to
/// drain keeps its place in a fleet whose activation requires the gate, instead of being refused by the registry and
/// serving nothing (re-review of 39157608, Major 1).
#[must_use]
pub fn listed(keys: bool) -> Vec<FlowControlCapability> {
    BUILT.into_iter().filter(|_| keys).collect()
}

/// One of the two flow-control capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FlowControlCapability {
    /// `concurrencyKeys`: a definition may declare `concurrency` (US02).
    ConcurrencyKeys,
    /// `rateLimits`: a definition may declare `rateLimit` (US07).
    RateLimits,
}

impl FlowControlCapability {
    pub const ALL: [Self; 2] = [Self::ConcurrencyKeys, Self::RateLimits];

    /// The name of the IR capability (ADR-0032 decision 8).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConcurrencyKeys => "concurrencyKeys",
            Self::RateLimits => "rateLimits",
        }
    }

    /// The instance capability of the registry that a whole fleet must honour before it is used.
    #[must_use]
    pub const fn registry(self) -> Capability {
        match self {
            Self::ConcurrencyKeys => Capability::FlowControlConcurrencyV1,
            Self::RateLimits => Capability::FlowControlRateV1,
        }
    }

    /// The IR field it allows.
    #[must_use]
    pub const fn field(self) -> &'static str {
        match self {
            Self::ConcurrencyKeys => "concurrency",
            Self::RateLimits => "rateLimit",
        }
    }

    #[must_use]
    pub const fn of(kind: GateKind) -> Self {
        match kind {
            GateKind::Concurrency => Self::ConcurrencyKeys,
            GateKind::RateLimit => Self::RateLimits,
        }
    }
}

/// The capabilities a definition needs: one for each kind of gate it declares, at the top level or on any node.
#[must_use]
pub fn required(definition: &WorkflowDefinition) -> BTreeSet<FlowControlCapability> {
    let levels = std::iter::once(definition.run_flow_control())
        .chain(definition.all_nodes().map(|node| node.flow_control()));
    let mut needed = BTreeSet::new();
    for level in levels {
        if level.concurrency().is_some() {
            needed.insert(FlowControlCapability::ConcurrencyKeys);
        }
        if level.rate_limit().is_some() {
            needed.insert(FlowControlCapability::RateLimits);
        }
    }
    needed
}

/// Where the first declaration a capability allows stands: `None` for the top level, else the node.
#[must_use]
pub fn first_use(
    definition: &WorkflowDefinition,
    capability: FlowControlCapability,
) -> Option<Option<&sagat_domain::workflow::NodeId>> {
    let declares = |level: &sagat_domain::workflow::FlowControl| match capability {
        FlowControlCapability::ConcurrencyKeys => level.concurrency().is_some(),
        FlowControlCapability::RateLimits => level.rate_limit().is_some(),
    };
    if declares(definition.run_flow_control()) {
        return Some(None);
    }
    definition
        .all_nodes()
        .find(|node| declares(node.flow_control()))
        .map(|node| Some(node.id()))
}

/// The deployment's switches (`flow-control.concurrency-keys`, `flow-control.rate-limits`). Both off unless the
/// deployment says so: a document check that runs without a deployment sees the default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FlowControlSwitches {
    pub concurrency_keys: bool,
    pub rate_limits: bool,
}

impl FlowControlSwitches {
    #[must_use]
    pub const fn enabled(self, capability: FlowControlCapability) -> bool {
        match capability {
            FlowControlCapability::ConcurrencyKeys => self.concurrency_keys,
            FlowControlCapability::RateLimits => self.rate_limits,
        }
    }

    /// The first capability `definition` needs that this deployment has switched off.
    #[must_use]
    pub fn first_disabled(self, definition: &WorkflowDefinition) -> Option<FlowControlCapability> {
        required(definition)
            .into_iter()
            .find(|capability| !self.enabled(*capability))
    }
}

/// The refusals of flow control (`contracts/errors.md`, "REST refusals"): each one is an existing `Error.code` with its
/// own `details.reason`, and none names a key's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FlowControlRefusal {
    /// 429 `QUOTA_EXCEEDED` / `CONCURRENCY_QUEUE_FULL`: the key's waiter queue is full (a synchronous start).
    #[error("the queue of the concurrency key is full")]
    ConcurrencyQueueFull,
    /// 429 `QUOTA_EXCEEDED` / `RATE_LIMIT_QUEUE_FULL`: the key's buffer, or the namespace's `queued_runs`, is full for
    /// a start that would be buffered.
    #[error("the buffer of the rate-limit key is full")]
    RateLimitQueueFull,
    /// 429 `QUOTA_EXCEEDED` / `FLOW_CONTROL_LIMIT`: a namespace ceiling of flow control (`limit` names it).
    #[error("a flow-control limit of the namespace was reached: {limit}")]
    Limit { limit: &'static str },
    /// 422 `VALIDATION_FAILED` / `FLOW_CONTROL_ACTIVATION_PENDING`: the capability is not activated for the fleet.
    #[error("the {} capability is not activated for the fleet yet", capability.as_str())]
    ActivationPending { capability: FlowControlCapability },
    /// 422 `VALIDATION_FAILED` / `CAPABILITY_DISABLED`: a start of a published version whose capability the deployment
    /// has switched off since (a rollback): existing runs drain, no new one starts.
    #[error("the {} capability is disabled by this deployment", capability.as_str())]
    CapabilityDisabled { capability: FlowControlCapability },
    /// 503 `UNAVAILABLE` / `BINARY_CAPABILITY_MISSING` (retryable): this process does not run the gate a recorded
    /// requirement asks for, so it does not admit the run (018's code, reused).
    #[error("this process does not run the flow-control gate")]
    BinaryCapabilityMissing,
    /// 409 `CONFLICT` / `CONCURRENCY_PERMIT_UNAVAILABLE` (018's code): a reset of a run whose definition gates its start
    /// found no free permit; a reset never queues, so nothing was created (FR-022-018).
    #[error("no permit of the concurrency key is free for the reset")]
    PermitUnavailable,
    /// 409 `CONFLICT` / `RATE_LIMIT_SLOT_UNAVAILABLE` (registered in 018's contract, D022-069): a reset of a run whose
    /// definition takes a slot of a rate-limit key at its start found no free slot in the key's window; a reset never
    /// waits, so nothing was created (FR-022-041). Given before the permit's when the start takes both (FR-022-052).
    #[error("no slot of the rate-limit key is free for the reset")]
    SlotUnavailable,
}

impl FlowControlRefusal {
    /// The `details.reason` of the refusal.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::ConcurrencyQueueFull => "CONCURRENCY_QUEUE_FULL",
            Self::RateLimitQueueFull => "RATE_LIMIT_QUEUE_FULL",
            Self::Limit { .. } => "FLOW_CONTROL_LIMIT",
            Self::ActivationPending { .. } => "FLOW_CONTROL_ACTIVATION_PENDING",
            Self::CapabilityDisabled { .. } => "CAPABILITY_DISABLED",
            Self::BinaryCapabilityMissing => crate::instances::REASON_CAPABILITY_MISSING,
            Self::PermitUnavailable => "CONCURRENCY_PERMIT_UNAVAILABLE",
            Self::SlotUnavailable => "RATE_LIMIT_SLOT_UNAVAILABLE",
        }
    }
}

/// The keys of one gated step, evaluated by the application for the readiness gate (feature 022, T28): the canonical
/// value of each gate the step's node declares, at most [`sagat_domain::workflow::flow_control::MAX_KEY_BYTES`] bytes. A gate without a
/// key expression has the single key of its group (`null`). The adapter digests and files them under the keys'
/// locks; the values never leave the transaction (D1168).
#[derive(Clone, PartialEq, Eq)]
pub struct StepGateKeys {
    pub node: sagat_domain::workflow::NodeId,
    /// The attempt the decision is for: the step's attempt plus one.
    pub attempt: u32,
    pub concurrency: Option<Vec<u8>>,
    pub rate: Option<Vec<u8>>,
}

/// A key's values are run data: only their sizes are shown.
impl std::fmt::Debug for StepGateKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StepGateKeys")
            .field("node", &self.node)
            .field("attempt", &self.attempt)
            .field("concurrency_len", &self.concurrency.as_ref().map(Vec::len))
            .field("rate_len", &self.rate.as_ref().map(Vec::len))
            .finish()
    }
}

/// The keys of the gates a run's start declares, evaluated by the application over the run's input when the start is
/// decided (feature 022, T10 and T19 of the plan, T022-022: at its admission, a delay's release or a reset): the canonical value of each, at most
/// [`sagat_domain::workflow::flow_control::MAX_KEY_BYTES`] bytes; a gate without a key expression has [`SINGLE_KEY`].
/// The adapter digests and files them under the keys' locks; the values never leave the transaction (D1168).
#[derive(Clone, PartialEq, Eq)]
pub struct RunGateKeys {
    pub concurrency: Option<Vec<u8>>,
    pub rate: Option<Vec<u8>>,
    /// Why the start is decided now, and so what may be decided (the domain's question, carried unchanged).
    pub cause: sagat_domain::execution::StartCause,
}

/// A key's values are run data: only their sizes are shown.
impl std::fmt::Debug for RunGateKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunGateKeys")
            .field("concurrency_len", &self.concurrency.as_ref().map(Vec::len))
            .field("rate_len", &self.rate.as_ref().map(Vec::len))
            .field("cause", &self.cause)
            .field("cause", &self.cause)
            .finish()
    }
}

/// The readiness gate's answer for the steps of one round (T28): the grants or waits of every gate of the steps it
/// decided, or, when a namespace ceiling refuses some of them (a new key beyond the live-keys ceiling, a ceiling of
/// zero: `FLOW_CONTROL_LIMIT`, G1199), only those refusals: nothing is filed then, and the next round asks again for the
/// others.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StepGateAnswer {
    pub decisions: Vec<sagat_domain::execution::GateEvent>,
    pub refused: Vec<(
        sagat_domain::workflow::NodeId,
        sagat_domain::execution::FailureCode,
    )>,
}

/// The canonical value of the single key of a group whose declaration has no key expression.
pub const SINGLE_KEY: &[u8] = b"null";

/// A key whose waiters the grant routine may serve now (feature 022, T26, ADR-0089 decision 3): a step key with
/// waiters whose next-eligible hint has come, on a home shard this replica holds. Its digest is a keyed digest, never a
/// value (D1168); `hint` is the hint as it was read, so a pass that grants nothing may clear it only if no release
/// moved it since (early, never late). Its `Debug` prints the key's label, never the digest: the full digest opens the
/// listing's position and is what a forged cursor would need (keys reviews of 2026-11-20, security minor 1).
#[derive(Clone, PartialEq, Eq)]
pub struct GateKey {
    pub tenant_id: sagat_domain::identity::TenantId,
    pub namespace_id: sagat_domain::identity::NamespaceId,
    pub kind: GateKind,
    /// Whose waiters the key queues: runs' starts (`Run`) or steps (`Step`); the scope is part of the key's identity.
    pub level: sagat_domain::workflow::GateLevel,
    pub group: sagat_domain::workflow::ResolvedGroup,
    pub digest: [u8; 32],
    pub shard: sagat_domain::sharding::Shard,
    pub hint: time::OffsetDateTime,
    /// The instant of the statement that selected the key: at or after its hint, at or before every count its pass makes,
    /// so a rest computed from it is early and never late (decision D022-066 item 3).
    pub selected_at: time::OffsetDateTime,
}

impl std::fmt::Debug for GateKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GateKey")
            .field("tenant_id", &self.tenant_id)
            .field("namespace_id", &self.namespace_id)
            .field("kind", &self.kind)
            .field("level", &self.level)
            .field("group", &self.group)
            .field(
                "label",
                &sagat_domain::execution::KeyLabel::of_digest(&self.digest).as_str(),
            )
            .field("shard", &self.shard)
            .field("hint", &self.hint)
            .field("selected_at", &self.selected_at)
            .finish()
    }
}

/// A queued step of a key, or a queued run's start (`node` is `None`), or one whose queue deadline passed (T26, T27).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateWaiter {
    pub tenant_id: sagat_domain::identity::TenantId,
    pub run_id: sagat_domain::execution::RunId,
    pub node: Option<sagat_domain::workflow::NodeId>,
}

/// How many waiters of one key one pass tries (D022-015, ADR-0089 decision 3, plan "grant pass": at most 256 grants per
/// key, so a flood on one key does not hold back the keys behind it).
pub const GRANTS_PER_KEY_PER_PASS: u32 = 256;

#[cfg(test)]
mod tests {
    use super::*;

    /// A gate key's `Debug` (logged by the grant pass) names its label, never its full digest (keys reviews of
    /// 2026-11-20, security minor 1).
    #[test]
    fn a_gate_keys_debug_prints_its_label_and_never_its_digest() {
        let digest: [u8; 32] = std::array::from_fn(|i| u8::try_from(0xa0 + i).unwrap_or(0));
        let key = GateKey {
            tenant_id: sagat_domain::identity::TenantId::generate(),
            namespace_id: sagat_domain::identity::NamespaceId::generate(),
            kind: GateKind::Concurrency,
            level: sagat_domain::workflow::GateLevel::Step,
            group: sagat_domain::workflow::ResolvedGroup::parse("providers").expect("a group"),
            digest,
            shard: sagat_domain::sharding::Shard::new(3).expect("a shard"),
            hint: time::OffsetDateTime::UNIX_EPOCH,
            selected_at: time::OffsetDateTime::UNIX_EPOCH,
        };
        let printed = format!("{key:?}");
        assert!(printed.contains("a0a1a2a3a4a5"), "the label: {printed}");
        // the seventh byte on, in the spellings a derived or hexadecimal Debug would use
        assert!(!printed.contains("a6a7a8"), "{printed}");
        assert!(!printed.contains("166, 167, 168"), "{printed}");
    }
}
