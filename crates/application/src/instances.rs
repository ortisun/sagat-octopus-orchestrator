//! The instance-capability registry (feature 018, `FR-018-052`; ADR-0050 decision 9; used by features 029 and 031).
//!
//! Every serving process, each control-plane and each execution-engine replica (the one-shot commands such as
//! `migrate`, `provision-tenant` and `cleanup-secrets` do not register), announces what its binary can do and what its
//! configuration runs before it serves, keeps announcing that it is alive, and refuses to serve while the platform
//! records a requirement for its kind that it cannot honour. A feature that needs every such process to honour it is
//! activated by an operator, in one transaction that verifies the live fleet and records the requirement (the adapter's
//! `activate`); a process learns of the requirement the next time its heartbeat reports that the requirement epoch moved.
//!
//! This module holds what has no database in it: the closed vocabulary (kinds and capability values), the rule that
//! decides which required capabilities a process lacks, the gate that the API and the passes consult, and the
//! [`Member`] that registers, beats and leaves. [`InstanceStore`] is the port the PostgreSQL adapter implements.
//!
//! # Ownership of a row
//!
//! Each process holds a random [`InstanceToken`] that it sends with every call; the registry stores only its hash and
//! touches a row only for the token that matches, so another process that knows the instance id (any holder of the
//! runtime role can read every id) cannot refresh, delete or rewrite the row. What this does not close (a holder of the
//! runtime role can still fill the table or hold the registration lock open) is gap G2000.
//!
//! # Fail mode, stated once
//!
//! - The registry cannot be reached when the process starts, or refuses it: the process does not serve (its gate is
//!   closed) and asks again every second. A table of 1,000 live rows is neither: the check of capabilities has passed
//!   by then, so the process serves, loudly, and is not counted.
//! - A heartbeat fails while the process serves: it is logged and tried again at the next beat, never fatal.
//!   - If the failure is a **plain connection failure** ([`StoreError::DatabaseUnavailable`]) the process keeps serving:
//!     it needs the database for everything it does, so a registry that cannot be reached is a database that cannot be
//!     reached, and stopping the fleet for it would turn a short outage into a longer one. After 30 s it no longer
//!     counts as live, an activation may record a requirement it lacks, and its first successful beat closes its gate.
//!   - Any **other** failure for longer than the liveness window (a lock or statement timeout on its own row, a
//!     permission or function error) closes the gate: the process would be invisible to activations and would never
//!     learn of an epoch change while the database works. The next successful beat decides again.
//! - A requirement recorded while the process serves is learned at the next beat: at most one heartbeat period (10 s,
//!   jittered up to 12 s) plus the time of the call, and only by a process that was not live (silent for 30 s) at the
//!   activation, since an activation refuses while a live process lacks the capability.
//! - The last successful beat is kept in the gate; readiness fails when it is older than the liveness window, whatever
//!   the gate says.

use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::entropy::{Entropy, SystemEntropy};
use crate::error::StoreError;

/// How often a process says it is alive (jittered by the caller).
pub const HEARTBEAT: Duration = Duration::from_secs(10);
/// A process is live when it was seen within this long, on the database's clock. The value is the database's
/// (the functions of migration `20270113000000_instance_registry` hold it); a test keeps the two equal.
pub const LIVENESS: Duration = Duration::from_secs(30);

/// The settings of a process's membership. Not read from the environment: the liveness window the activation judges by
/// is the database's, so a setting that moved the heartbeat would only be a way to make a healthy process look dead.
/// A test sets the interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistrySettings {
    /// How often the process says it is alive (the loop jitters it by 20 %).
    pub heartbeat: Duration,
}

impl Default for RegistrySettings {
    fn default() -> Self {
        Self {
            heartbeat: HEARTBEAT,
        }
    }
}

/// The reason an API call or a probe is refused with while the gate is closed for want of a capability
/// (`details.reason` of the `503`).
pub const REASON_CAPABILITY_MISSING: &str = "BINARY_CAPABILITY_MISSING";
/// The closed reasons of the refusal counter besides the one above.
pub const REASON_REGISTRY_FULL: &str = "REGISTRY_FULL";
pub const REASON_REGISTRY_UNREACHABLE: &str = "REGISTRY_UNREACHABLE";

/// The kind of process an instance is. A closed set that only grows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum InstanceKind {
    ControlPlane,
    ExecutionEngine,
}

impl InstanceKind {
    pub const ALL: [Self; 2] = [Self::ControlPlane, Self::ExecutionEngine];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ControlPlane => "CONTROL_PLANE",
            Self::ExecutionEngine => "EXECUTION_ENGINE",
        }
    }

    /// The kind a stored token names, if this build knows it (a kind added by a newer build is not an error here).
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == token)
    }
}

/// A capability value. Owned by the feature that introduces it (`data-model.md`, rule 5); the schema holds the same
/// closed list, and a test keeps the two, and the data model, equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Capability {
    /// Feature 018: the engine fires the schedules, and the legacy cron pass may be handed over to them.
    SchedulesV1,
    /// Feature 029: the control plane enforces restricted folders and operation lists.
    AccessNarrowingV1,
    /// Feature 031: the control plane and the engine read and write the payload encoding.
    PayloadEncodingV1,
}

impl Capability {
    pub const ALL: [Self; 3] = [
        Self::SchedulesV1,
        Self::AccessNarrowingV1,
        Self::PayloadEncodingV1,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SchedulesV1 => "schedules-v1",
            Self::AccessNarrowingV1 => "access-narrowing-v1",
            Self::PayloadEncodingV1 => "payload-encoding-v1",
        }
    }

    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|capability| capability.as_str() == token)
    }
}

/// The secret a process keeps to itself to prove that a row is its own: 32 random bytes, new at every start. The registry
/// stores only the SHA-256 of it. It is never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct InstanceToken([u8; 32]);

impl InstanceToken {
    /// A new token from `entropy` (the operating system's, in a process).
    #[must_use]
    pub fn generate(entropy: &impl Entropy) -> Self {
        Self(entropy.token())
    }

    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for InstanceToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InstanceToken(<redacted>)")
    }
}

/// What a process says about itself when it registers.
#[derive(Debug, Clone, Copy)]
pub struct Registration<'a> {
    /// Random per process start: two processes never share one, and a restart is a new instance.
    pub instance: Uuid,
    /// Proves the row is this process's: a row is registered again, refreshed or deleted only for the token it holds.
    pub token: &'a InstanceToken,
    pub kind: InstanceKind,
    /// An engine's `scheduler_members.owner`, for correlation only.
    pub scheduler_owner: Option<&'a str>,
    pub binary_version: &'a str,
    pub capabilities: &'a [Capability],
}

/// What registering came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registered {
    /// The row is written; `epoch` is the requirement epoch the check was made under.
    Serving { epoch: i64 },
    /// A requirement for the kind names capabilities the process lacks; nothing was written.
    CapabilityMissing { epoch: i64, missing: Vec<String> },
    /// The check of capabilities passed and the table holds its most live rows: nothing was written, the process may
    /// serve and is not counted.
    Full { epoch: i64 },
}

/// What a heartbeat found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heartbeat {
    /// Whether the instance's row was there (it is not when the day rule or an operator removed it).
    pub found: bool,
    pub epoch: i64,
    /// The live instances by kind token, as stored (a kind this build does not know is carried, not an error).
    pub live: Vec<(String, u32)>,
}

/// A recorded requirement, as stored: the tokens are read as written, so a capability or a kind added by a newer
/// build is carried and compared, never a failure to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    pub capability: String,
    pub kinds: Vec<String>,
}

/// The requirements and the epoch they were read under, from one statement.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Requirements {
    pub epoch: i64,
    pub required: Vec<Requirement>,
}

/// The capabilities `requirements` names for `kind` that `own` does not list, in the order they are stored. Pure: the
/// whole rule of "may this binary serve", which the register function repeats in SQL under the epoch lock.
#[must_use]
pub fn missing_for(
    kind: InstanceKind,
    own: &[Capability],
    requirements: &Requirements,
) -> Vec<String> {
    requirements
        .required
        .iter()
        .filter(|requirement| requirement.kinds.iter().any(|k| k == kind.as_str()))
        .filter(|requirement| !own.iter().any(|c| c.as_str() == requirement.capability))
        .map(|requirement| requirement.capability.clone())
        .collect()
}

/// The port of the registry that a serving process uses. Implemented by `sagat-infra-postgres`.
pub trait InstanceStore: Send + Sync {
    /// One transaction: take the requirement epoch `FOR SHARE`, check the requirements for the kind, and, if the process
    /// honours them and the table has room, write the row (a second call with the same instance and token refreshes
    /// it; the same instance under another token or kind is refused, the row untouched).
    fn register(
        &self,
        registration: &Registration<'_>,
    ) -> impl Future<Output = Result<Registered, StoreError>> + Send;

    /// Sets `last_seen` of the instance's own row (the one the token opens) to the database's clock, deletes the rows
    /// silent for a day, and reports the epoch and the live counts. A row the token does not open is a row not found:
    /// it is not touched.
    fn heartbeat(
        &self,
        instance: Uuid,
        token: &InstanceToken,
    ) -> impl Future<Output = Result<Heartbeat, StoreError>> + Send;

    /// Deletes the instance's own row (a clean stop). Whether it was there for this token.
    fn deregister(
        &self,
        instance: Uuid,
        token: &InstanceToken,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// The requirement epoch.
    fn epoch(&self) -> impl Future<Output = Result<i64, StoreError>> + Send;

    /// The requirements and the epoch, read together.
    fn requirements(&self) -> impl Future<Output = Result<Requirements, StoreError>> + Send;
}

/// Whether a process serves. Shared between the membership that decides it and the places that obey it (the API's
/// admission, the engine's passes and probes). It starts closed: nothing serves before it registered. It also keeps
/// the time of the last answer of the registry, which readiness reads.
#[derive(Debug)]
pub struct ServingGate {
    open: AtomicBool,
    epoch: AtomicI64,
    missing: Mutex<Vec<String>>,
    created: Instant,
    last_heard: Mutex<Option<Instant>>,
}

impl Default for ServingGate {
    fn default() -> Self {
        Self::closed()
    }
}

impl ServingGate {
    /// A closed gate that has seen no epoch.
    #[must_use]
    pub fn closed() -> Self {
        Self {
            open: AtomicBool::new(false),
            epoch: AtomicI64::new(-1),
            missing: Mutex::new(Vec::new()),
            created: Instant::now(),
            last_heard: Mutex::new(None),
        }
    }

    /// An open gate, for a process or a test that has no registry.
    #[must_use]
    pub fn opened() -> Self {
        let gate = Self::closed();
        gate.open(0);
        gate
    }

    /// The requirements of the kind are honoured as of `epoch`.
    pub fn open(&self, epoch: i64) {
        self.missing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.epoch.store(epoch, Ordering::Release);
        self.open.store(true, Ordering::Release);
    }

    /// The process lacks `missing` as of `epoch` (`missing` empty: the gate is closed for want of an answer, not of a
    /// capability).
    pub fn close(&self, epoch: i64, missing: Vec<String>) {
        *self.missing.lock().unwrap_or_else(PoisonError::into_inner) = missing;
        self.epoch.store(epoch, Ordering::Release);
        self.open.store(false, Ordering::Release);
    }

    /// One atomic load: what every request asks.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }

    /// The epoch the gate was last decided under (`-1` before the first decision).
    #[must_use]
    pub fn epoch(&self) -> i64 {
        self.epoch.load(Ordering::Acquire)
    }

    /// The capabilities that keep the gate closed (empty while it is open, and when it is closed for another reason).
    #[must_use]
    pub fn missing(&self) -> Vec<String> {
        self.missing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The registry answered now.
    pub fn heard(&self) {
        self.heard_at(Instant::now());
    }

    /// The registry answered at `at`.
    pub fn heard_at(&self, at: Instant) {
        *self
            .last_heard
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(at);
    }

    /// How long since the registry last answered (since the gate was made, if it never has), as of `now`.
    #[must_use]
    pub fn silent_for(&self, now: Instant) -> Duration {
        let since = self
            .last_heard
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .unwrap_or(self.created);
        now.saturating_duration_since(since)
    }

    /// Whether the registry has been silent for longer than a process stays live: readiness fails (the gate rule is
    /// its own, see the fail mode above).
    #[must_use]
    pub fn beat_is_stale(&self, now: Instant) -> bool {
        self.silent_for(now) > LIVENESS
    }
}

/// Why a process may not serve. A closed set of reasons, which is also the closed label of the refusal counter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The platform requires capabilities this process lacks.
    CapabilityMissing(Vec<String>),
    /// The registry could not be asked (the process does not serve without it).
    Unavailable(StoreError),
}

impl Refusal {
    /// The closed token: `BINARY_CAPABILITY_MISSING` or `REGISTRY_UNREACHABLE`.
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        match self {
            Self::CapabilityMissing(_) => REASON_CAPABILITY_MISSING,
            Self::Unavailable(_) => REASON_REGISTRY_UNREACHABLE,
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CapabilityMissing(missing) => write!(
                f,
                "{REASON_CAPABILITY_MISSING}: the platform requires {} and this process lacks it; upgrade it, or start it with the passes that provide it, or stop it",
                missing.join(", ")
            ),
            Self::Unavailable(error) => write!(f, "{REASON_REGISTRY_UNREACHABLE}: {error}"),
        }
    }
}

impl std::error::Error for Refusal {}

/// Whether the registry counts the process that registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Counting {
    /// Its row is written: an activation sees it.
    Counted,
    /// The table holds its most live rows: the process may serve, and an activation cannot see it.
    NotCounted,
}

/// What one beat did to the gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Beat {
    /// The epoch had not moved (or the requirements it moved to are honoured): the gate is as it was or open.
    Serving,
    /// The gate is closed after this beat, or stays closed.
    Refused,
}

/// A process's membership of the registry: it registers before serving, beats while it serves, and leaves.
#[derive(Debug)]
pub struct Member<S> {
    store: S,
    instance: Uuid,
    token: InstanceToken,
    kind: InstanceKind,
    binary_version: String,
    capabilities: Vec<Capability>,
    scheduler_owner: Option<String>,
    gate: Arc<ServingGate>,
}

impl<S: InstanceStore> Member<S> {
    /// A member with a closed gate and a token of its own. `instance` is random per process start; `capabilities` is
    /// what the process can do: the binary's, narrowed by the passes its configuration turns on.
    #[must_use]
    pub fn new(
        store: S,
        instance: Uuid,
        kind: InstanceKind,
        binary_version: impl Into<String>,
        capabilities: &[Capability],
        scheduler_owner: Option<String>,
    ) -> Self {
        Self::with_token(
            store,
            instance,
            InstanceToken::generate(&SystemEntropy),
            kind,
            binary_version,
            capabilities,
            scheduler_owner,
        )
    }

    /// As [`Self::new`] with the token given (a test that plays the process and the hostile one).
    #[must_use]
    pub fn with_token(
        store: S,
        instance: Uuid,
        token: InstanceToken,
        kind: InstanceKind,
        binary_version: impl Into<String>,
        capabilities: &[Capability],
        scheduler_owner: Option<String>,
    ) -> Self {
        let mut capabilities = capabilities.to_vec();
        capabilities.sort();
        capabilities.dedup();
        Self {
            store,
            instance,
            token,
            kind,
            binary_version: binary_version.into(),
            capabilities,
            scheduler_owner,
            gate: Arc::new(ServingGate::closed()),
        }
    }

    #[must_use]
    pub fn gate(&self) -> &Arc<ServingGate> {
        &self.gate
    }

    #[must_use]
    pub const fn instance(&self) -> Uuid {
        self.instance
    }

    #[must_use]
    pub const fn kind(&self) -> InstanceKind {
        self.kind
    }

    async fn register_once(&self) -> Result<Registered, StoreError> {
        let registered = self
            .store
            .register(&Registration {
                instance: self.instance,
                token: &self.token,
                kind: self.kind,
                scheduler_owner: self.scheduler_owner.as_deref(),
                binary_version: &self.binary_version,
                capabilities: &self.capabilities,
            })
            .await?;
        self.gate.heard();
        Ok(registered)
    }

    /// Registers, and opens the gate. To be called before the first request is served and before the first pass
    /// runs. A refusal leaves the gate closed and is counted. A full table is not a refusal: the check of capabilities
    /// has passed, so the process serves and is not counted, loudly.
    ///
    /// # Errors
    /// [`Refusal`] when the platform requires what this process lacks, or the registry cannot be asked.
    pub async fn register(&self) -> Result<Counting, Refusal> {
        let outcome = match self.register_once().await {
            Ok(Registered::Serving { epoch }) => {
                self.gate.open(epoch);
                Ok(Counting::Counted)
            }
            Ok(Registered::Full { epoch }) => {
                self.gate.open(epoch);
                count_refusal(REASON_REGISTRY_FULL);
                Ok(Counting::NotCounted)
            }
            Ok(Registered::CapabilityMissing { epoch, missing }) => {
                self.gate.close(epoch, missing.clone());
                let refusal = Refusal::CapabilityMissing(missing);
                count_refusal(refusal.reason());
                Err(refusal)
            }
            Err(error) => {
                let refusal = Refusal::Unavailable(error);
                count_refusal(refusal.reason());
                Err(refusal)
            }
        };
        self.observe(Instant::now());
        outcome
    }

    /// One heartbeat: says the process is alive and, when the epoch moved, reads the requirements again and decides
    /// the gate. A row that is gone (or not this process's) is registered again, with the whole check.
    ///
    /// # Errors
    /// The store could not be asked. The gate is left as it was: see the fail mode in the module documentation, and
    /// [`Self::fail_closed`] for the one case the caller closes it.
    pub async fn beat(&self) -> Result<Beat, StoreError> {
        let heartbeat = self.store.heartbeat(self.instance, &self.token).await?;
        self.gate.heard();
        self.publish_live(&heartbeat);
        let beat = self.decide(&heartbeat).await;
        self.observe(Instant::now());
        beat
    }

    async fn decide(&self, heartbeat: &Heartbeat) -> Result<Beat, StoreError> {
        if !heartbeat.found {
            return match self.register_once().await? {
                Registered::Serving { epoch } => {
                    self.reopen(epoch);
                    Ok(Beat::Serving)
                }
                Registered::CapabilityMissing { epoch, missing } => {
                    self.closing(epoch, missing);
                    Ok(Beat::Refused)
                }
                // Registering would have passed the check, so the process may go on serving; it is only not counted
                // until the table has room. The next beat tries again.
                Registered::Full { epoch } => {
                    count_refusal(REASON_REGISTRY_FULL);
                    self.reopen(epoch);
                    Ok(Beat::Serving)
                }
            };
        }
        if heartbeat.epoch == self.gate.epoch() {
            return Ok(self.current());
        }
        let requirements = self.store.requirements().await?;
        let missing = missing_for(self.kind, &self.capabilities, &requirements);
        if missing.is_empty() {
            self.reopen(requirements.epoch);
            Ok(Beat::Serving)
        } else {
            self.closing(requirements.epoch, missing);
            Ok(Beat::Refused)
        }
    }

    fn current(&self) -> Beat {
        if self.gate.is_open() {
            Beat::Serving
        } else {
            Beat::Refused
        }
    }

    fn reopen(&self, epoch: i64) {
        if !self.gate.is_open() {
            tracing::info!(
                instance = %self.instance,
                epoch,
                "the registry lets this process serve (it registered, or the platform no longer requires a capability it lacks)"
            );
        }
        self.gate.open(epoch);
    }

    fn closing(&self, epoch: i64, missing: Vec<String>) {
        if self.gate.is_open() || self.gate.missing() != missing {
            tracing::error!(
                instance = %self.instance,
                kind = self.kind.as_str(),
                epoch,
                capabilities = %missing.join(","),
                "{REASON_CAPABILITY_MISSING}: the platform requires what this process lacks; it stops serving until it is upgraded, restarted with the passes that provide it, or the requirement is removed"
            );
            count_refusal(REASON_CAPABILITY_MISSING);
        }
        self.gate.close(epoch, missing);
    }

    /// Closes the gate because the registry has failed for longer than the liveness window in a way that is not a plain
    /// connection failure: the process is invisible to activations and would never learn of an epoch change. The epoch
    /// is forgotten, so that the next beat that succeeds reads the requirements and decides again.
    pub fn fail_closed(&self) {
        if self.gate.is_open() {
            tracing::error!(
                instance = %self.instance,
                kind = self.kind.as_str(),
                "{REASON_REGISTRY_UNREACHABLE}: the registry has failed for longer than the liveness window and the failure is not a lost connection; this process stops serving until a heartbeat succeeds"
            );
            count_refusal(REASON_REGISTRY_UNREACHABLE);
        }
        self.gate.close(-1, Vec::new());
        self.observe(Instant::now());
    }

    /// The fleet's live counts, as this beat saw them. The same value from every process: read it with `max`.
    fn publish_live(&self, heartbeat: &Heartbeat) {
        for kind in InstanceKind::ALL {
            let live = heartbeat
                .live
                .iter()
                .find(|(token, _)| token == kind.as_str())
                .map_or(0, |(_, n)| *n);
            metrics::gauge!("serving_instances_live", "kind" => kind.as_str()).set(f64::from(live));
        }
    }

    /// The two gauges of this process: whether its gate is open, and how long since the registry last answered it.
    /// Called at every try, so a registry that stops answering shows as an age that grows.
    pub fn observe(&self, now: Instant) {
        metrics::gauge!("instance_registry_gate_open").set(if self.gate.is_open() {
            1.0
        } else {
            0.0
        });
        metrics::gauge!("instance_registry_last_beat_age_seconds")
            .set(self.gate.silent_for(now).as_secs_f64());
    }

    /// Deletes the row (a clean stop). A failure is logged and not returned: the row expires by itself.
    pub async fn leave(&self) {
        match self.store.deregister(self.instance, &self.token).await {
            Ok(_) => tracing::info!(instance = %self.instance, "left the instance registry"),
            Err(error) => tracing::warn!(
                instance = %self.instance,
                %error,
                "could not delete this instance from the registry; it expires by itself"
            ),
        }
    }
}

fn count_refusal(reason: &'static str) {
    metrics::counter!("instance_registry_refusals_total", "reason" => reason).increment(1);
}

/// What an operator asks to record: every live instance of `kinds` must list `capability`.
#[derive(Debug, Clone, Copy)]
pub struct Activation<'a> {
    pub capability: Capability,
    pub kinds: &'a [InstanceKind],
    pub actor: &'a sagat_domain::identity::Subject,
}

/// The outcome of an activation that was not refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activated {
    /// The requirement is recorded and the epoch is now `epoch`; the feature's own activation ran in the same
    /// transaction.
    Recorded { epoch: i64 },
    /// The same requirement was recorded before, and the feature's own activation with it: nothing was done.
    AlreadyRecorded,
}

/// The outcome of removing a requirement that was not refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removed {
    Removed {
        epoch: i64,
    },
    /// No such requirement was recorded: nothing was done.
    NotRecorded,
}

/// Why an activation or a removal did not happen.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChangeRefused {
    /// Live instances of the required kinds do not list the capability. Instance ids are not content: up to sixteen
    /// are named so that the operator can find the processes.
    #[error("{missing} live instance(s) of the required kinds lack the capability")]
    InstancesLackCapability { missing: u32, examples: Vec<Uuid> },
    /// The capability is recorded for other kinds; remove it first.
    #[error("the capability is already required of other kinds; remove it first")]
    KindsDiffer,
    /// The registry or the feature's own activation failed; nothing was recorded.
    #[error(transparent)]
    Store(#[from] StoreError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn requirement(capability: &str, kinds: &[&str]) -> Requirement {
        Requirement {
            capability: capability.to_owned(),
            kinds: kinds.iter().map(|k| (*k).to_owned()).collect(),
        }
    }

    fn requirements(epoch: i64, required: Vec<Requirement>) -> Requirements {
        Requirements { epoch, required }
    }

    #[test]
    fn the_closed_values_round_trip_and_nothing_else_parses() {
        for kind in InstanceKind::ALL {
            assert_eq!(InstanceKind::parse(kind.as_str()), Some(kind));
        }
        for capability in Capability::ALL {
            assert_eq!(Capability::parse(capability.as_str()), Some(capability));
        }
        assert_eq!(InstanceKind::parse("RUNNER"), None);
        assert_eq!(InstanceKind::parse("control_plane"), None);
        assert_eq!(Capability::parse("schedules-v2"), None);
        assert_eq!(Capability::parse(""), None);
        let tokens: std::collections::BTreeSet<_> =
            Capability::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(tokens.len(), Capability::ALL.len(), "no two share a token");
    }

    #[test]
    fn a_process_lacks_what_is_required_of_its_kind_and_it_does_not_list() {
        let required = requirements(
            3,
            vec![
                requirement("schedules-v1", &["EXECUTION_ENGINE"]),
                requirement("access-narrowing-v1", &["CONTROL_PLANE"]),
                requirement(
                    "payload-encoding-v1",
                    &["CONTROL_PLANE", "EXECUTION_ENGINE"],
                ),
            ],
        );
        assert_eq!(
            missing_for(InstanceKind::ExecutionEngine, &[], &required),
            vec!["schedules-v1", "payload-encoding-v1"],
            "an engine with nothing lacks the two required of engines, not the one for the control plane"
        );
        assert_eq!(
            missing_for(
                InstanceKind::ExecutionEngine,
                &[Capability::SchedulesV1],
                &required
            ),
            vec!["payload-encoding-v1"]
        );
        assert_eq!(
            missing_for(
                InstanceKind::ControlPlane,
                &[Capability::AccessNarrowingV1, Capability::PayloadEncodingV1],
                &required
            ),
            Vec::<String>::new()
        );
        assert_eq!(
            missing_for(InstanceKind::ControlPlane, &[], &Requirements::default()),
            Vec::<String>::new(),
            "nothing required, nothing missing"
        );
    }

    #[test]
    fn a_requirement_written_by_a_newer_build_is_missing_and_not_a_failure() {
        let required = requirements(
            9,
            vec![
                requirement("replay-v9", &["EXECUTION_ENGINE"]),
                requirement("schedules-v1", &["EXECUTION_ENGINE", "SIDECAR"]),
                requirement("future-v1", &["SIDECAR"]),
            ],
        );
        assert_eq!(
            missing_for(
                InstanceKind::ExecutionEngine,
                &[Capability::SchedulesV1],
                &required
            ),
            vec!["replay-v9"],
            "an unknown capability is lacking; an unknown kind in the list concerns no known kind"
        );
        assert_eq!(
            missing_for(InstanceKind::ControlPlane, &[], &required),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_gate_starts_closed_opens_and_closes_with_what_it_lacks() {
        let gate = ServingGate::closed();
        assert!(!gate.is_open());
        assert_eq!(gate.epoch(), -1);
        gate.open(4);
        assert!(gate.is_open());
        assert_eq!((gate.epoch(), gate.missing()), (4, Vec::new()));
        gate.close(5, vec!["schedules-v1".to_owned()]);
        assert!(!gate.is_open());
        assert_eq!(gate.epoch(), 5);
        assert_eq!(gate.missing(), vec!["schedules-v1"]);
        gate.open(6);
        assert!(gate.is_open());
        assert!(
            gate.missing().is_empty(),
            "opening forgets what was lacking"
        );
        assert!(ServingGate::opened().is_open());
    }

    #[test]
    fn the_gate_counts_the_silence_of_the_registry_from_its_last_answer() {
        let gate = ServingGate::closed();
        let now = Instant::now();
        // never answered: since the gate was made
        assert!(!gate.beat_is_stale(now));
        assert!(gate.beat_is_stale(now + LIVENESS + Duration::from_secs(1)));
        gate.heard_at(now);
        assert_eq!(
            gate.silent_for(now + Duration::from_secs(7)),
            Duration::from_secs(7)
        );
        assert!(
            !gate.beat_is_stale(now + LIVENESS),
            "30 s of silence is not beyond the window"
        );
        assert!(gate.beat_is_stale(now + LIVENESS + Duration::from_millis(1)));
        // an answer starts the count again
        gate.heard_at(now + Duration::from_secs(100));
        assert!(!gate.beat_is_stale(now + Duration::from_secs(120)));
        assert_eq!(gate.silent_for(now), Duration::ZERO, "never negative");
    }

    #[test]
    fn a_refusal_names_a_closed_reason_and_a_message_with_the_capability() {
        let missing = Refusal::CapabilityMissing(vec!["schedules-v1".to_owned()]);
        assert_eq!(missing.reason(), "BINARY_CAPABILITY_MISSING");
        assert!(missing.to_string().contains("schedules-v1"));
        assert!(missing.to_string().contains("BINARY_CAPABILITY_MISSING"));
        assert_eq!(
            Refusal::Unavailable(StoreError::DatabaseUnavailable).reason(),
            "REGISTRY_UNREACHABLE"
        );
        assert_eq!(REASON_REGISTRY_FULL, "REGISTRY_FULL");
    }

    #[test]
    fn a_token_is_never_printed_and_two_processes_never_share_one() {
        let first = InstanceToken::generate(&SystemEntropy);
        let second = InstanceToken::generate(&SystemEntropy);
        assert_ne!(first, second);
        assert_eq!(first.clone(), first);
        assert_eq!(format!("{first:?}"), "InstanceToken(<redacted>)");
        assert_eq!(InstanceToken::from_bytes([7; 32]).as_bytes(), &[7; 32]);
    }

    /// A registry in memory that answers what a test queued and remembers what it was asked.
    #[derive(Default)]
    struct Fake {
        registers: Mutex<VecDeque<Result<Registered, StoreError>>>,
        heartbeats: Mutex<VecDeque<Result<Heartbeat, StoreError>>>,
        reads: Mutex<VecDeque<Result<Requirements, StoreError>>>,
        read_count: Mutex<usize>,
        register_count: Mutex<usize>,
        left: Mutex<usize>,
        /// The token of every call, to prove the process sends the one it registered with.
        tokens: Mutex<Vec<[u8; 32]>>,
    }

    impl InstanceStore for Arc<Fake> {
        async fn register(
            &self,
            registration: &Registration<'_>,
        ) -> Result<Registered, StoreError> {
            *self.register_count.lock().expect("lock") += 1;
            self.tokens
                .lock()
                .expect("lock")
                .push(*registration.token.as_bytes());
            self.registers
                .lock()
                .expect("lock")
                .pop_front()
                .expect("a queued registration")
        }
        async fn heartbeat(&self, _: Uuid, token: &InstanceToken) -> Result<Heartbeat, StoreError> {
            self.tokens.lock().expect("lock").push(*token.as_bytes());
            self.heartbeats
                .lock()
                .expect("lock")
                .pop_front()
                .expect("a queued heartbeat")
        }
        async fn deregister(&self, _: Uuid, token: &InstanceToken) -> Result<bool, StoreError> {
            self.tokens.lock().expect("lock").push(*token.as_bytes());
            *self.left.lock().expect("lock") += 1;
            Ok(true)
        }
        async fn epoch(&self) -> Result<i64, StoreError> {
            Ok(0)
        }
        async fn requirements(&self) -> Result<Requirements, StoreError> {
            *self.read_count.lock().expect("lock") += 1;
            self.reads
                .lock()
                .expect("lock")
                .pop_front()
                .expect("a queued read")
        }
    }

    fn member(fake: &Arc<Fake>, capabilities: &[Capability]) -> Member<Arc<Fake>> {
        Member::new(
            Arc::clone(fake),
            Uuid::from_u128(7),
            InstanceKind::ExecutionEngine,
            "0.3.0",
            capabilities,
            Some("engine-a".to_owned()),
        )
    }

    fn beat(found: bool, epoch: i64) -> Result<Heartbeat, StoreError> {
        Ok(Heartbeat {
            found,
            epoch,
            live: vec![("EXECUTION_ENGINE".to_owned(), 2)],
        })
    }

    #[tokio::test]
    async fn registering_opens_the_gate_under_the_epoch_it_was_checked_at() {
        let fake = Arc::new(Fake::default());
        fake.registers
            .lock()
            .expect("lock")
            .push_back(Ok(Registered::Serving { epoch: 5 }));
        let member = member(&fake, &[Capability::SchedulesV1]);
        assert!(!member.gate().is_open(), "closed until it registered");
        assert_eq!(member.register().await, Ok(Counting::Counted));
        assert!(member.gate().is_open());
        assert_eq!(member.gate().epoch(), 5);
        assert!(
            !member.gate().beat_is_stale(Instant::now()),
            "the registry answered"
        );
    }

    #[tokio::test]
    async fn a_full_table_at_the_start_is_served_loudly_and_not_counted() {
        let fake = Arc::new(Fake::default());
        fake.registers
            .lock()
            .expect("lock")
            .push_back(Ok(Registered::Full { epoch: 9 }));
        let member = member(&fake, &[]);
        assert_eq!(
            member.register().await,
            Ok(Counting::NotCounted),
            "the check of capabilities passed before the table was looked at"
        );
        assert!(member.gate().is_open());
        assert_eq!(member.gate().epoch(), 9);
    }

    #[tokio::test]
    async fn a_refused_registration_leaves_the_gate_closed_and_says_why() {
        let fake = Arc::new(Fake::default());
        let member = member(&fake, &[]);
        fake.registers.lock().expect("lock").extend([
            Ok(Registered::CapabilityMissing {
                epoch: 8,
                missing: vec!["schedules-v1".to_owned()],
            }),
            Err(StoreError::DatabaseUnavailable),
        ]);
        assert_eq!(
            member.register().await,
            Err(Refusal::CapabilityMissing(vec!["schedules-v1".to_owned()]))
        );
        assert!(!member.gate().is_open());
        assert_eq!(member.gate().missing(), vec!["schedules-v1"]);
        assert_eq!(
            member.register().await,
            Err(Refusal::Unavailable(StoreError::DatabaseUnavailable))
        );
        assert!(!member.gate().is_open());
    }

    #[tokio::test]
    async fn every_call_of_a_process_carries_the_token_it_registered_with() {
        let fake = Arc::new(Fake::default());
        fake.registers.lock().expect("lock").extend([
            Ok(Registered::Serving { epoch: 1 }),
            Ok(Registered::Serving { epoch: 1 }),
        ]);
        let member = member(&fake, &[]);
        member.register().await.expect("registered");
        fake.heartbeats
            .lock()
            .expect("lock")
            .extend([beat(true, 1), beat(false, 1)]);
        member.beat().await.expect("beat");
        member.beat().await.expect("beat that registers again");
        member.leave().await;
        let tokens = fake.tokens.lock().expect("lock").clone();
        assert_eq!(
            tokens.len(),
            5,
            "register, beat, beat, register again, leave"
        );
        assert!(tokens.iter().all(|token| *token == tokens[0]));
        let other = self::member(&fake, &[]);
        fake.registers
            .lock()
            .expect("lock")
            .push_back(Ok(Registered::Serving { epoch: 1 }));
        other.register().await.expect("registered");
        assert_ne!(
            *fake.tokens.lock().expect("lock").last().expect("a token"),
            tokens[0],
            "another process holds another token"
        );
    }

    #[tokio::test]
    async fn a_beat_that_finds_the_same_epoch_reads_nothing_more() {
        let fake = Arc::new(Fake::default());
        fake.registers
            .lock()
            .expect("lock")
            .push_back(Ok(Registered::Serving { epoch: 5 }));
        let member = member(&fake, &[]);
        member.register().await.expect("registered");
        fake.heartbeats
            .lock()
            .expect("lock")
            .extend([beat(true, 5), beat(true, 5)]);
        assert_eq!(member.beat().await, Ok(Beat::Serving));
        assert_eq!(member.beat().await, Ok(Beat::Serving));
        assert_eq!(*fake.read_count.lock().expect("lock"), 0);
    }

    #[tokio::test]
    async fn an_epoch_that_moved_reads_the_requirements_once_and_closes_the_gate_when_one_is_lacked()
     {
        let fake = Arc::new(Fake::default());
        fake.registers
            .lock()
            .expect("lock")
            .push_back(Ok(Registered::Serving { epoch: 5 }));
        let member = member(&fake, &[Capability::SchedulesV1]);
        member.register().await.expect("registered");
        fake.heartbeats
            .lock()
            .expect("lock")
            .extend([beat(true, 6), beat(true, 6), beat(true, 7)]);
        fake.reads.lock().expect("lock").extend([
            Ok(requirements(
                6,
                vec![requirement("payload-encoding-v1", &["EXECUTION_ENGINE"])],
            )),
            Ok(requirements(7, vec![])),
        ]);
        assert_eq!(member.beat().await, Ok(Beat::Refused));
        assert!(!member.gate().is_open());
        assert_eq!(member.gate().missing(), vec!["payload-encoding-v1"]);
        assert_eq!(member.gate().epoch(), 6);
        // the same epoch again: nothing is read, the gate stays closed
        assert_eq!(member.beat().await, Ok(Beat::Refused));
        assert_eq!(*fake.read_count.lock().expect("lock"), 1);
        // the operator removed the requirement: the epoch moves and the gate opens again
        assert_eq!(member.beat().await, Ok(Beat::Serving));
        assert!(member.gate().is_open());
        assert_eq!(member.gate().epoch(), 7);
    }

    #[tokio::test]
    async fn a_failed_beat_changes_nothing_and_a_missing_row_is_registered_again_with_the_whole_check()
     {
        let fake = Arc::new(Fake::default());
        fake.registers
            .lock()
            .expect("lock")
            .push_back(Ok(Registered::Serving { epoch: 5 }));
        let member = member(&fake, &[]);
        member.register().await.expect("registered");
        fake.heartbeats.lock().expect("lock").extend([
            Err(StoreError::DatabaseUnavailable),
            beat(false, 5),
            beat(false, 9),
        ]);
        assert_eq!(
            member.beat().await,
            Err(StoreError::DatabaseUnavailable),
            "the error is the caller's to log"
        );
        assert!(member.gate().is_open(), "serving goes on");
        // the row is gone and registering again finds a requirement
        fake.registers.lock().expect("lock").extend([
            Ok(Registered::CapabilityMissing {
                epoch: 9,
                missing: vec!["schedules-v1".to_owned()],
            }),
            Ok(Registered::Serving { epoch: 9 }),
        ]);
        assert_eq!(member.beat().await, Ok(Beat::Refused));
        assert!(!member.gate().is_open());
        // and when the requirement is gone, registering again lets it serve
        fake.heartbeats
            .lock()
            .expect("lock")
            .push_back(beat(false, 10));
        assert_eq!(member.beat().await, Ok(Beat::Serving));
        assert!(member.gate().is_open());
    }

    #[tokio::test]
    async fn a_full_table_does_not_stop_a_process_that_was_serving() {
        let fake = Arc::new(Fake::default());
        fake.registers
            .lock()
            .expect("lock")
            .push_back(Ok(Registered::Serving { epoch: 5 }));
        let member = member(&fake, &[]);
        member.register().await.expect("registered");
        fake.heartbeats
            .lock()
            .expect("lock")
            .push_back(beat(false, 5));
        fake.registers
            .lock()
            .expect("lock")
            .push_back(Ok(Registered::Full { epoch: 5 }));
        assert_eq!(
            member.beat().await,
            Ok(Beat::Serving),
            "registering again would have passed the check: the process may go on serving"
        );
        assert!(member.gate().is_open());
    }

    #[tokio::test]
    async fn a_process_that_failed_closed_and_finds_the_table_full_when_it_registers_again_serves_again()
     {
        let fake = Arc::new(Fake::default());
        fake.registers
            .lock()
            .expect("lock")
            .push_back(Ok(Registered::Serving { epoch: 5 }));
        let member = member(&fake, &[]);
        member.register().await.expect("registered");
        member.fail_closed();
        assert!(!member.gate().is_open());
        fake.heartbeats
            .lock()
            .expect("lock")
            .push_back(beat(false, 5));
        fake.registers
            .lock()
            .expect("lock")
            .push_back(Ok(Registered::Full { epoch: 5 }));
        assert_eq!(member.beat().await, Ok(Beat::Serving));
        assert!(member.gate().is_open(), "the check of capabilities passed");
        assert_eq!(member.gate().epoch(), 5);
    }

    #[tokio::test]
    async fn a_beat_that_the_registry_answers_is_an_answer_for_the_readiness_of_the_process() {
        let fake = Arc::new(Fake::default());
        fake.registers
            .lock()
            .expect("lock")
            .push_back(Ok(Registered::Serving { epoch: 1 }));
        let member = member(&fake, &[]);
        member.register().await.expect("registered");
        // the registration was heard long ago; a beat is heard now
        member
            .gate()
            .heard_at(Instant::now() - Duration::from_secs(40));
        assert!(member.gate().beat_is_stale(Instant::now()));
        fake.heartbeats
            .lock()
            .expect("lock")
            .push_back(beat(true, 1));
        member.beat().await.expect("beat");
        assert!(
            !member.gate().beat_is_stale(Instant::now()),
            "a heartbeat that succeeds ends the silence"
        );
    }

    #[tokio::test]
    async fn a_registration_is_an_answer_of_the_registry_for_the_readiness_of_the_process() {
        let fake = Arc::new(Fake::default());
        fake.registers
            .lock()
            .expect("lock")
            .push_back(Ok(Registered::Serving { epoch: 1 }));
        let member = member(&fake, &[]);
        // nothing has been heard for a long time (the process started and the registry could not be asked)
        member
            .gate()
            .heard_at(Instant::now() - Duration::from_secs(40));
        assert!(member.gate().beat_is_stale(Instant::now()));
        member.register().await.expect("registered");
        assert!(
            !member.gate().beat_is_stale(Instant::now()),
            "a registration that succeeds ends the silence"
        );
    }

    #[tokio::test]
    async fn an_epoch_that_went_back_is_read_again_and_not_trusted() {
        // a database restored from a backup answers an epoch lower than the one the gate was decided under
        let fake = Arc::new(Fake::default());
        fake.registers
            .lock()
            .expect("lock")
            .push_back(Ok(Registered::Serving { epoch: 5 }));
        let member = member(&fake, &[]);
        member.register().await.expect("registered");
        fake.heartbeats
            .lock()
            .expect("lock")
            .push_back(beat(true, 3));
        fake.reads
            .lock()
            .expect("lock")
            .push_back(Ok(requirements(3, vec![])));
        assert_eq!(member.beat().await, Ok(Beat::Serving));
        assert_eq!(*fake.read_count.lock().expect("lock"), 1);
        assert_eq!(
            member.gate().epoch(),
            3,
            "decided again under the epoch it found"
        );
    }

    #[tokio::test]
    async fn failing_closed_forgets_the_epoch_so_that_the_next_beat_that_succeeds_decides_again() {
        let fake = Arc::new(Fake::default());
        fake.registers
            .lock()
            .expect("lock")
            .push_back(Ok(Registered::Serving { epoch: 5 }));
        let member = member(&fake, &[]);
        member.register().await.expect("registered");
        member.fail_closed();
        assert!(!member.gate().is_open());
        assert!(
            member.gate().missing().is_empty(),
            "closed for want of an answer, not of a capability"
        );
        assert_eq!(member.gate().epoch(), -1);
        // the registry answers again with the epoch it had: the requirements are read and the gate opens
        fake.heartbeats
            .lock()
            .expect("lock")
            .push_back(beat(true, 5));
        fake.reads
            .lock()
            .expect("lock")
            .push_back(Ok(requirements(5, vec![])));
        assert_eq!(member.beat().await, Ok(Beat::Serving));
        assert!(member.gate().is_open());
        assert_eq!(member.gate().epoch(), 5);
        // and it closes with the capability when the requirement is there
        member.fail_closed();
        fake.heartbeats
            .lock()
            .expect("lock")
            .push_back(beat(true, 5));
        fake.reads.lock().expect("lock").push_back(Ok(requirements(
            5,
            vec![requirement("schedules-v1", &["EXECUTION_ENGINE"])],
        )));
        assert_eq!(member.beat().await, Ok(Beat::Refused));
        assert_eq!(member.gate().missing(), vec!["schedules-v1"]);
    }

    /// Runs `body` with the metrics of the module recorded into a recorder of its own, and returns what it rendered.
    fn recorded<F: std::future::Future<Output = ()>>(body: impl FnOnce() -> F) -> String {
        let (metrics, recorder) = sagat_telemetry::Metrics::detached(Duration::from_secs(60));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a runtime");
        metrics::with_local_recorder(&recorder, || runtime.block_on(body()));
        metrics.render()
    }

    #[test]
    fn a_beat_publishes_the_live_count_of_every_kind_and_a_missing_kind_is_zero() {
        let page = recorded(|| async {
            let fake = Arc::new(Fake::default());
            fake.registers
                .lock()
                .expect("lock")
                .push_back(Ok(Registered::Serving { epoch: 1 }));
            let member = member(&fake, &[]);
            member.register().await.expect("registered");
            fake.heartbeats
                .lock()
                .expect("lock")
                .push_back(Ok(Heartbeat {
                    found: true,
                    epoch: 1,
                    live: vec![("EXECUTION_ENGINE".to_owned(), 3), ("RUNNER".to_owned(), 9)],
                }));
            member.beat().await.expect("beat");
        });
        assert!(
            page.contains("serving_instances_live{kind=\"EXECUTION_ENGINE\"} 3"),
            "{page}"
        );
        assert!(
            page.contains("serving_instances_live{kind=\"CONTROL_PLANE\"} 0"),
            "{page}"
        );
        assert!(
            !page.contains("RUNNER"),
            "a kind this build does not know is not a label: the labels are a closed set"
        );
    }

    #[test]
    fn the_gate_and_the_age_of_the_last_beat_are_gauges_of_the_process() {
        let page = recorded(|| async {
            let fake = Arc::new(Fake::default());
            fake.registers
                .lock()
                .expect("lock")
                .push_back(Ok(Registered::Serving { epoch: 1 }));
            let member = member(&fake, &[]);
            // closed before it registered: 0, and the age counts from the start
            member.observe(Instant::now());
            member.register().await.expect("registered");
            member
                .gate()
                .heard_at(Instant::now() - Duration::from_secs(12));
            member.observe(Instant::now());
            member.fail_closed();
        });
        // the last value of each gauge: closed again, 12 s since the answer
        assert!(page.contains("instance_registry_gate_open 0"), "{page}");
        let age: f64 = page
            .lines()
            .find_map(|l| l.strip_prefix("instance_registry_last_beat_age_seconds "))
            .and_then(|v| v.trim().parse().ok())
            .expect("the age gauge");
        assert!((11.0..60.0).contains(&age), "{age} s: {page}");
    }

    #[test]
    fn the_gate_gauge_is_one_while_the_process_serves() {
        let page = recorded(|| async {
            let fake = Arc::new(Fake::default());
            fake.registers
                .lock()
                .expect("lock")
                .push_back(Ok(Registered::Serving { epoch: 1 }));
            member(&fake, &[]).register().await.expect("registered");
        });
        assert!(page.contains("instance_registry_gate_open 1"), "{page}");
    }

    #[test]
    fn a_refusal_is_counted_under_its_closed_reason_once_when_a_running_process_learns_of_it() {
        let page = recorded(|| async {
            let fake = Arc::new(Fake::default());
            fake.registers.lock().expect("lock").extend([
                Ok(Registered::Serving { epoch: 1 }),
                Ok(Registered::Full { epoch: 2 }),
            ]);
            let member = member(&fake, &[]);
            member.register().await.expect("registered");
            // a requirement is learned, twice at the same epoch: counted once
            fake.heartbeats
                .lock()
                .expect("lock")
                .extend([beat(true, 2), beat(true, 2)]);
            fake.reads.lock().expect("lock").push_back(Ok(requirements(
                2,
                vec![requirement("schedules-v1", &["EXECUTION_ENGINE"])],
            )));
            member.beat().await.expect("beat");
            member.beat().await.expect("beat");
            // the row is gone and the table is full
            fake.heartbeats
                .lock()
                .expect("lock")
                .push_back(beat(false, 2));
            member.beat().await.expect("beat");
        });
        assert!(
            page.contains(
                "instance_registry_refusals_total{reason=\"BINARY_CAPABILITY_MISSING\"} 1"
            ),
            "{page}"
        );
        assert!(
            page.contains("instance_registry_refusals_total{reason=\"REGISTRY_FULL\"} 1"),
            "{page}"
        );
    }

    #[test]
    fn a_requirement_that_is_still_lacked_at_a_later_epoch_is_counted_once_and_a_new_one_is_counted_again()
     {
        let page = recorded(|| async {
            let fake = Arc::new(Fake::default());
            fake.registers
                .lock()
                .expect("lock")
                .push_back(Ok(Registered::Serving { epoch: 1 }));
            let member = member(&fake, &[]);
            member.register().await.expect("registered");
            fake.heartbeats.lock().expect("lock").extend([
                beat(true, 2),
                beat(true, 3),
                beat(true, 4),
            ]);
            fake.reads.lock().expect("lock").extend([
                Ok(requirements(
                    2,
                    vec![requirement("schedules-v1", &["EXECUTION_ENGINE"])],
                )),
                // the epoch moved again (another requirement, for another kind): this process lacks the same one
                Ok(requirements(
                    3,
                    vec![requirement("schedules-v1", &["EXECUTION_ENGINE"])],
                )),
                // and now it lacks one more
                Ok(requirements(
                    4,
                    vec![
                        requirement("schedules-v1", &["EXECUTION_ENGINE"]),
                        requirement("payload-encoding-v1", &["EXECUTION_ENGINE"]),
                    ],
                )),
            ]);
            for _ in 0..3 {
                member.beat().await.expect("beat");
            }
        });
        assert!(
            page.contains(
                "instance_registry_refusals_total{reason=\"BINARY_CAPABILITY_MISSING\"} 2"
            ),
            "counted when the lack began and when it changed, not at every epoch: {page}"
        );
    }

    #[test]
    fn a_refused_registration_and_a_failing_closed_are_counted_under_their_reasons() {
        let page = recorded(|| async {
            let fake = Arc::new(Fake::default());
            fake.registers.lock().expect("lock").extend([
                Ok(Registered::Full { epoch: 1 }),
                Err(StoreError::DatabaseUnavailable),
                Ok(Registered::CapabilityMissing {
                    epoch: 1,
                    missing: vec!["schedules-v1".to_owned()],
                }),
            ]);
            let member = member(&fake, &[]);
            for _ in 0..3 {
                let _ = member.register().await;
            }
            // closing the gate for want of an answer is counted once, while it was open
            fake.registers
                .lock()
                .expect("lock")
                .push_back(Ok(Registered::Serving { epoch: 1 }));
            member.register().await.expect("serving again");
            member.fail_closed();
            member.fail_closed();
        });
        for (reason, count) in [
            ("REGISTRY_FULL", 1),
            ("REGISTRY_UNREACHABLE", 2),
            ("BINARY_CAPABILITY_MISSING", 1),
        ] {
            assert!(
                page.contains(&format!(
                    "instance_registry_refusals_total{{reason=\"{reason}\"}} {count}"
                )),
                "{reason}: {page}"
            );
        }
    }

    #[tokio::test]
    async fn leaving_deletes_the_row_once() {
        let fake = Arc::new(Fake::default());
        let member = member(&fake, &[]);
        member.leave().await;
        assert_eq!(*fake.left.lock().expect("lock"), 1);
    }
}
