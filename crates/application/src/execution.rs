//! Ports and the generic engine for durable execution (feature 002).
//!
//! A run changes only inside a [`RunTx`]: the store locks the run's row, loads its immutable
//! definition and current state, the [`Engine`] runs one domain command against them (resolving
//! any condition or correlation key through the [`ExpressionEvaluator`]), and the store persists the
//! resulting journal events, the step projections, timers/subscriptions, payloads and outbox rows
//! and commits them together (`FR-002-002`, `FR-002-003`). If the transaction is dropped without a
//! commit, nothing happened.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::execution::{
    CorrelationKey, Evaluation, ExecutionError, ExecutionInstanceId, FailureCode, Progress,
    Resolution, RunEvent, RunId, RunState, RunStatus, StopReason,
};
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::workflow::{ActivityName, CancelPolicy, Expression, NodeId, WorkflowDefinition};
use time::OffsetDateTime;

use sagat_domain::audit::AuditRecord;

use crate::blobs::BlobCommit;
use crate::definitions::{Idempotency, IrCodec, IrError};
use crate::error::StoreError;
use crate::secrets::{SecretInputs, StoredSecret};
use sagat_domain::secrets::SecretName;

/// Largest payload (run input, step output) kept inline; bigger ones need a blob store, which is not
/// built yet (research.md, gap G19).
pub const MAX_PAYLOAD_BYTES: usize = 256 * 1024;

/// Most runs one tree may hold: a root run and all its descendants. Depth alone does not bound a
/// tree (a parallel node can start hundreds of children per level), so the whole tree has a budget.
pub const MAX_TREE_RUNS: usize = 1024;

/// What the refusal names when a tree is full (`StoreError::QuotaExceeded::what`); the scheduler
/// tells it apart from the quotas of a namespace or a tenant, which fail the child differently.
pub const TREE_LIMIT: &str = "runs in one child-run tree";

/// Estimated memory the parsed definitions of one process may occupy (the engine and the control
/// plane each run in a few hundred megabytes).
pub const DEFAULT_CACHE_BYTES: usize = 64 * 1024 * 1024;

/// Most output bytes a run may keep. Every lease receives the outputs of the steps before it, so
/// without a limit the sum of many individually allowed outputs would be copied into every task.
pub const MAX_RUN_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

/// Deepest nesting of child runs: a run started by the API is at depth 0, its children at 1, and so
/// on. A recursive workflow therefore ends in a failed step, never in an unbounded tree.
pub const MAX_CHILD_DEPTH: u8 = 8;

/// The id of the child run a `child` step of `parent` reserves. Derived, not random, so a retried
/// reservation names the same run and a crash between journaling it and creating it is repaired by
/// creating exactly that run.
#[must_use]
pub fn child_run_id(parent: RunId, node: &NodeId) -> RunId {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("child/{parent}/{node}").as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    RunId::new(uuid::Uuid::from_bytes(bytes))
}

/// The stable identity of a step: derived from the version, the run and the node, so every
/// instance computes the same value and a hash collision would be caught by the unique key.
#[must_use]
pub fn step_id(version: WorkflowVersionId, run: RunId, node: &NodeId) -> uuid::Uuid {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("{version}/{run}//{node}").as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    // Mark it as a version-8 (custom) UUID with the RFC 4122 variant.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes)
}

/// Stable opaque id of a logical step in a nested scope. Root work retains its historical id
/// byte-for-byte. The nonroot encoding is domain-separated and length-delimited, so a path cannot
/// collide by concatenating user-controlled node names. Attempts never enter this derivation.
#[must_use]
pub fn instance_step_id(
    version: WorkflowVersionId,
    run: RunId,
    instance: &ExecutionInstanceId,
) -> uuid::Uuid {
    use sha2::{Digest, Sha256};

    if instance.path().is_root() {
        return step_id(version, run, instance.node());
    }
    let mut digest = Sha256::new();
    digest.update(b"sagat.step.instance.scoped.v1\0");
    digest.update(version.as_uuid().as_bytes());
    digest.update(run.as_uuid().as_bytes());
    digest.update([instance.path().depth() as u8]);
    for segment in instance.path().segments() {
        let name = segment.scope_node().as_str().as_bytes();
        digest.update([name.len() as u8]);
        digest.update(name);
        digest.update((segment.iteration_index() as u16).to_be_bytes());
    }
    let name = instance.node().as_str().as_bytes();
    digest.update([name.len() as u8]);
    digest.update(name);
    let hash = digest.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&hash[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes)
}

/// The stable identity of the compensation of `forward` in a workflow version: the same for every run
/// of that version, so a provider that is asked to undo `reserve` can recognise the request, and the
/// plan stored with the version names the same ids a run uses. With the run id it makes the
/// idempotency key of one compensation (`compensation_key`).
pub fn compensation_id(version: WorkflowVersionId, forward: &NodeId) -> uuid::Uuid {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("{version}//compensation//{forward}").as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    // A version-8 (custom) UUID with the RFC 4122 variant, like `step_id`.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes)
}

/// The idempotency key of the compensation of `forward` in one run: stable per run, step and action
/// (`docs/architecture/sagas.md`, rule 3), so asking again, after a lost response or a restart, can
/// never undo the effect twice.
#[must_use]
pub fn compensation_key(version: WorkflowVersionId, run: RunId, forward: &NodeId) -> String {
    format!("compensation:{}:{run}", compensation_id(version, forward))
}

/// When the compensations of one provider operation stop being asked for (FR-003-003).
///
/// A provider that is down would otherwise be called again by every run that has something to
/// undo, each on its own retry schedule. After `threshold` attempts in a row that ended without
/// success, no compensation of that activity is leased for `open_for`; then one probe is, and its
/// outcome closes the breaker or reopens it for twice as long, up to `max_open`. A step that waits
/// is not failing: nothing is lost by waiting, only the retry budget of the step is untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BreakerPolicy {
    /// Consecutive failures that open the breaker; zero turns it off.
    pub threshold: u32,
    pub open_for: std::time::Duration,
    pub max_open: std::time::Duration,
}

impl Default for BreakerPolicy {
    fn default() -> Self {
        Self {
            threshold: 5,
            open_for: std::time::Duration::from_secs(30),
            max_open: std::time::Duration::from_secs(300),
        }
    }
}

impl BreakerPolicy {
    /// A breaker that never opens.
    pub const OFF: Self = Self {
        threshold: 0,
        open_for: std::time::Duration::ZERO,
        max_open: std::time::Duration::ZERO,
    };
}

/// The data expressions can read.
#[derive(Debug, Clone, Default)]
pub struct RunData {
    pub run_id: Option<RunId>,
    pub input: Vec<u8>,
    /// Catalog-backed run input only; a caller-provided JSON lookalike never sets this identity.
    pub input_blob: Option<sagat_domain::blobs::BlobId>,
    /// Outputs of the steps that succeeded, by node id.
    pub outputs: BTreeMap<NodeId, Vec<u8>>,
}

/// An expression failed to evaluate. The detail is for logs; it never reaches clients.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("expression failed: {0}")]
pub struct ExpressionFailure(pub String);

/// What a reconciliation found out, as the activity reported it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileAnswer {
    pub verdict: sagat_domain::saga::ReconcileVerdict,
    /// For an effect that happened: the output the forward step would have had, if the activity
    /// provides it.
    pub output: Option<Vec<u8>>,
}

/// Evaluates the expressions of a workflow over a run's data (implemented by `sagat-infra-ir`).
///
/// Evaluation is synchronous and bounded: it has no side effects and cannot loop.
pub trait ExpressionEvaluator: Send + Sync {
    /// # Errors
    /// [`ExpressionFailure`] for a runtime error (a missing field, a type error).
    fn condition(&self, expression: &Expression, data: &RunData)
    -> Result<bool, ExpressionFailure>;

    /// # Errors
    /// [`ExpressionFailure`] when the result is not a usable correlation key.
    fn correlation_key(
        &self,
        expression: &Expression,
        data: &RunData,
    ) -> Result<CorrelationKey, ExpressionFailure>;

    /// Renders an idempotency-key template for `node`.
    ///
    /// # Errors
    /// [`ExpressionFailure`] when a part fails or the result is too long.
    fn idempotency_key(
        &self,
        template: &Expression,
        data: &RunData,
        node: &NodeId,
    ) -> Result<String, ExpressionFailure>;

    /// The input of a child run: `expression` evaluated over the parent's data, or the parent's own
    /// input when the node defines none. It must be a JSON object, like every run input.
    ///
    /// # Errors
    /// [`ExpressionFailure`] when the expression fails or does not yield an object.
    fn child_input(
        &self,
        expression: Option<&Expression>,
        data: &RunData,
    ) -> Result<Vec<u8>, ExpressionFailure>;

    /// The JSON document a worker receives as the input of `node`.
    fn task_input(&self, data: &RunData, node: &NodeId) -> Vec<u8>;

    /// Reads the answer of a reconciliation activity: a JSON object
    /// `{"effect": "applied" | "not_applied" | "unknown", "output": <JSON>}` where `output` is
    /// optional and only meaningful for `applied`.
    ///
    /// # Errors
    /// [`ExpressionFailure`] when the output is not such an object.
    fn reconcile_answer(&self, output: &[u8]) -> Result<ReconcileAnswer, ExpressionFailure>;

    /// The document a run started by an event, or a step woken by one, is given:
    /// `{"event": {id, source, type, subject, correlationKey, occurredAt, receivedAt}, "data": ...}`.
    /// The same document is what a trigger's filter is evaluated over, as `input`.
    fn event_document(&self, event: &crate::dispatch::EventView<'_>) -> Vec<u8>;

    /// Whether `filter` holds for an event: a condition over `event` (its envelope) and `data`.
    /// `document` is what [`Self::event_document`] produced.
    ///
    /// # Errors
    /// [`ExpressionFailure`] when the expression cannot be evaluated over this event (a field it
    /// names is missing, a type does not fit): the event then does not match.
    fn event_filter(&self, filter: &Expression, document: &[u8])
    -> Result<bool, ExpressionFailure>;

    /// Whether `bytes` is a well-formed JSON document of any kind.
    fn is_json(&self, bytes: &[u8]) -> bool;

    /// Whether `bytes` is a well-formed JSON object (a run input must be one).
    fn is_json_object(&self, bytes: &[u8]) -> bool;
}

/// The run that started a child, and what becomes of the child when the parent stops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentLink {
    pub run: RunId,
    pub node: NodeId,
    pub cancel_policy: CancelPolicy,
}

/// A child run reserved by a `child` step: the id, and the input it will start with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildSpawn {
    pub node: NodeId,
    pub child: RunId,
    pub input: Vec<u8>,
}

/// Everything about a run that does not change after it is created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunInfo {
    pub run_id: RunId,
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    pub workflow_id: WorkflowId,
    pub version_id: WorkflowVersionId,
    pub started_by: String,
    pub correlation_id: Option<String>,
    pub trace_id: Option<String>,
    /// The ended run this one was started again from (`RERUN_AS_NEW`), when it was.
    pub rerun_of: Option<RunId>,
    /// The span the run's own span hangs from: the request that started it, or the request that
    /// published the event that did. None for a child run (its parent's span is its parent) and for
    /// a run created before spans were recorded.
    pub parent_span: Option<String>,
    pub deadline_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    /// The run that started this one, when it is a child (`FR-002-009`).
    pub parent: Option<ParentLink>,
    /// How deeply nested this run is: 0 for a run started through the API.
    pub depth: u8,
    /// The run at the top of the tree this run belongs to (itself for a run started through the API).
    pub root: RunId,
}

/// A run as read by the API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRecord {
    pub info: RunInfo,
    pub status: RunStatus,
    pub version: u64,
    pub stop_reason: Option<StopReason>,
    pub paused: bool,
    pub updated_at: OffsetDateTime,
    pub finished_at: Option<OffsetDateTime>,
}

/// Who caused the events of one transaction (stored with every journal entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    /// A tenant principal (`sub`).
    Principal(String),
    /// A worker session.
    Worker(String),
    /// The engine itself (timers, reconciler, deadlines).
    System(&'static str),
}

impl Actor {
    /// The text stored in the journal.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Principal(sub) => format!("user:{sub}"),
            Self::Worker(id) => format!("worker:{id}"),
            Self::System(name) => format!("system:{name}"),
        }
    }
}

/// What the engine asks the store to write besides the journal events themselves.
#[derive(Debug, Clone)]
pub struct Effects {
    pub actor: Actor,
    pub trace_id: Option<String>,
    /// The outputs of steps that finished in this transaction: what a worker returned, or the
    /// payload of the signal that woke a wait.
    pub outputs: Vec<(NodeId, Vec<u8>)>,
    /// The lease granted to a worker by a `step.leased` event of this transaction.
    pub lease: Option<LeaseGrant>,
    /// A heartbeat that renews the lease of one step.
    pub renewal: Option<LeaseRenewal>,
    /// The audit record of an operator action, committed with the change it describes.
    pub audit: Option<AuditRecord>,
    /// The idempotency record of the command, with the result a replay should return.
    pub idempotency: Option<(Idempotency, String)>,
    /// `READY` steps to write a fresh wake-up notification for (their notification was lost).
    pub rehint: Vec<NodeId>,
    /// Child runs reserved in this transaction; the engine fills it while resolving `child` steps.
    pub children: Vec<ChildSpawn>,
    /// Blobs the outputs above are descriptors of: committed with them, or the transaction fails.
    pub blobs: Vec<BlobCommit>,
}

impl Effects {
    #[must_use]
    pub const fn by(actor: Actor, trace_id: Option<String>) -> Self {
        Self {
            actor,
            trace_id,
            outputs: Vec::new(),
            lease: None,
            renewal: None,
            audit: None,
            idempotency: None,
            rehint: Vec::new(),
            children: Vec::new(),
            blobs: Vec::new(),
        }
    }
}

/// When a lease ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeaseDeadlines {
    /// The renewable expiry: a heartbeat pushes it out.
    pub expires_at: OffsetDateTime,
    /// The attempt's own deadline (the node's `timeout`), which no renewal extends.
    pub attempt_deadline_at: Option<OffsetDateTime>,
}

/// The lease handed to a worker. Only the hash of the token is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseGrant {
    pub node: NodeId,
    pub token_hash: Vec<u8>,
    pub worker_id: String,
    /// Added to the database clock: a lease lasts this long unless renewed.
    pub ttl: std::time::Duration,
    /// The step's `timeout`, if any: renewals never extend an attempt past it.
    pub attempt_timeout: Option<std::time::Duration>,
}

/// A heartbeat: push the lease expiry out by `ttl` (capped by the attempt deadline).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseRenewal {
    pub node: NodeId,
    pub ttl: std::time::Duration,
}

/// One locked run, mutated in memory and written back in a single transaction.
pub trait RunTx: Send {
    fn info(&self) -> &RunInfo;

    /// The database clock when the transaction began: every deadline is computed from it, so all
    /// instances agree regardless of their own clocks.
    fn now(&self) -> OffsetDateTime;

    /// The immutable definition and the mutable state, borrowed together.
    fn parts(&mut self) -> (&WorkflowDefinition, &mut RunState);

    /// The oldest worker protocol that can safely receive work from this run. This is decided
    /// from durable catalog provenance while the run is locked, never by inspecting JSON that may
    /// happen to resemble a protocol-specific descriptor.
    fn required_worker_protocol_minor(
        &mut self,
    ) -> impl Future<Output = Result<u32, StoreError>> + Send;

    /// When the lease held on `node` expires (the renewal capped by the attempt deadline), or
    /// `None` when no lease is held.
    fn lease_expiry(
        &mut self,
        node: &NodeId,
    ) -> impl Future<Output = Result<Option<OffsetDateTime>, StoreError>> + Send;

    /// The two times at which the lease held on `node` ends (`None` when no lease is held): the
    /// renewable expiry and the attempt's own deadline, if it has one.
    fn lease_deadlines(
        &mut self,
        node: &NodeId,
    ) -> impl Future<Output = Result<Option<LeaseDeadlines>, StoreError>> + Send;

    /// Whether `owner` still holds the shard lease `lease`, as of the database's clock. It takes
    /// the shard's row in shared mode, held to the end of the transaction, and a takeover of the
    /// shard needs it exclusively: a transaction that found the lease good finishes before the
    /// shard changes hands, and one that starts after finds the new generation. This is the fence
    /// of a scheduler action (`FR-010-001`).
    fn holds_shard(
        &mut self,
        owner: &str,
        lease: crate::shards::Lease,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Whether the worker identity `subject` is being drained in this run's namespace. It takes
    /// the drain's lock in shared mode, held to the end of the transaction, and a drain takes it
    /// exclusively: a drain does not return while a transaction that found the worker serving is
    /// still deciding whether to lease, so no work is handed out after the drain has returned.
    fn worker_drained(
        &mut self,
        subject: &str,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Locks and revalidates the worker session that is about to receive a lease, then returns the
    /// exact number of leases it can still hold. The lock is retained to commit, so claims on
    /// different runs and replicas cannot race the session's `max_inflight` limit. Call this after
    /// the run/drain fences and before activity admission to keep one global lock order.
    fn worker_session_room(
        &mut self,
        session_id: uuid::Uuid,
        token_hash: &[u8],
        subject: &str,
    ) -> impl Future<Output = Result<u32, StoreError>> + Send;

    /// The activity patterns `subject` is bound to in this run's namespace. Read on the
    /// transaction's own connection: a caller that holds one connection and waits for a second
    /// from the same pool can starve it under load.
    fn bound_patterns(
        &mut self,
        subject: &str,
    ) -> impl Future<Output = Result<Vec<String>, StoreError>> + Send;

    /// The declared `names` as they are stored for the publisher of this run's version, with the
    /// publisher's current role: what a worker's resolution is decided on. Read on the
    /// transaction's own connection.
    fn secret_inputs(
        &mut self,
        names: &[SecretName],
    ) -> impl Future<Output = Result<SecretInputs, StoreError>> + Send;

    /// Every stored version of the declared `names` (a handful each), newest first, with the
    /// secrets that no longer exist simply absent: what an output is searched for. A value handed
    /// out earlier in the attempt may have been replaced since, so the current one is not enough.
    fn secret_versions(
        &mut self,
        names: &[SecretName],
    ) -> impl Future<Output = Result<Vec<StoredSecret>, StoreError>> + Send;

    /// Whether `token_hash` is the hash of the lease currently held on `node` *and* that lease has
    /// not run out by the database clock. A late report is accepted until the scheduler reclaims
    /// the attempt, to save work; a capability to read secrets ends with the lease instead.
    fn lease_unexpired(
        &mut self,
        node: &NodeId,
        token_hash: &[u8],
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Whether the circuit breaker of the compensations of `activity` lets one be leased now.
    ///
    /// Closed: yes. Open: no. Half open (the window is over): yes for exactly one caller, whose
    /// lease is the probe, and no for the others until the probe reports. Asking is not free of
    /// effect: the caller that is told yes is the probe, and must lease.
    fn compensation_admitted(
        &mut self,
        activity: &ActivityName,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Reserves room under the exact concurrency quota of `activity` for this transaction.
    /// `false` leaves the step ready. A `true` caller must lease before committing; dropping the
    /// transaction releases the reservation lock without consuming a place.
    fn activity_admitted(
        &mut self,
        activity: &ActivityName,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Freezes the retention of every script execution this new lease can create. For a forward
    /// attempt this includes its immutable compensation and reconciliation activities; for either
    /// derived obligation it inherits the pin recorded by the originating forward attempt.
    ///
    /// `false` leaves a new forward step ready when the current retention policy is zero. The
    /// adapter keeps the selected pins in this transaction and adds them to the immutable
    /// `step.leased` journal detail when the lease is persisted.
    fn script_execution_admitted(
        &mut self,
        node: &NodeId,
        activity: &ActivityName,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Whether `token_hash` is the hash of the lease currently held on `node`.
    fn lease_matches(
        &mut self,
        node: &NodeId,
        token_hash: &[u8],
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// The result stored by an earlier command with the same idempotency key, if any. A different
    /// request under the same key is [`StoreError::IdempotencyKeyReused`].
    fn replay(
        &mut self,
        idempotency: &Idempotency,
    ) -> impl Future<Output = Result<Option<String>, StoreError>> + Send;

    /// The run's input and the outputs of its finished steps (loaded once per transaction).
    fn data(&mut self) -> impl Future<Output = Result<&RunData, StoreError>> + Send;

    /// Makes outputs that are about to be persisted visible to this transaction's evaluations: a
    /// condition that follows a step that just completed must see what that step returned.
    fn stage_outputs(
        &mut self,
        outputs: &[(NodeId, Vec<u8>)],
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Appends `events` to the journal and updates every projection (steps, timers, subscriptions,
    /// payloads, the run row, the outbox) in this transaction. `state` already includes `events`.
    fn persist(
        &mut self,
        events: &[RunEvent],
        effects: Effects,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Records, in this transaction, that an event was delivered to a consumer of this run (a step
    /// it wakes, or the trigger that starts it). `true` when the delivery is new; `false` when the
    /// event had been delivered to that consumer before, in which case nothing may be done again.
    /// A rolled-back transaction leaves no delivery behind, so the event can still reach the
    /// consumer later.
    fn record_delivery(
        &mut self,
        delivery: &crate::dispatch::DeliveryRef,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Commits. Dropping the transaction instead rolls everything back.
    fn commit(self) -> impl Future<Output = Result<(), StoreError>> + Send;
}

/// Why an engine call failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EngineError {
    /// The domain refused the command (stale caller, illegal transition, run finished, ...).
    #[error(transparent)]
    Execution(#[from] ExecutionError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Runs domain commands against a locked run.
#[derive(Debug, Clone)]
pub struct Engine<E> {
    evaluator: E,
}

impl<E: ExpressionEvaluator> Engine<E> {
    pub const fn new(evaluator: E) -> Self {
        Self { evaluator }
    }

    #[must_use]
    pub const fn evaluator(&self) -> &E {
        &self.evaluator
    }

    /// Executes `command` against the transaction's run, resolves whatever the planner needs to
    /// evaluate, and persists the whole outcome. Nothing is committed here.
    ///
    /// On an error the transaction must be dropped (rolled back): the in-memory state may already
    /// reflect events that were never written.
    ///
    /// # Errors
    /// [`EngineError::Execution`] when the domain refuses the command, [`EngineError::Store`] when
    /// loading data or persisting fails.
    pub async fn run<T, Tx, F>(
        &self,
        tx: &mut Tx,
        mut effects: Effects,
        command: F,
    ) -> Result<T, EngineError>
    where
        Tx: RunTx,
        F: FnOnce(
            &WorkflowDefinition,
            &mut RunState,
            OffsetDateTime,
        ) -> Result<(T, Progress), ExecutionError>,
    {
        let now = tx.now();
        if !effects.outputs.is_empty() {
            tx.stage_outputs(&effects.outputs).await?;
        }
        let (value, mut progress) = {
            let (def, state) = tx.parts();
            command(def, state, now)?
        };
        let mut events = std::mem::take(&mut progress.events);
        // A condition or a correlation key can chain (a condition leading to a wait), but each
        // round consumes a node of the definition, so this is bounded by the graph.
        let mut rounds = 0usize;
        while let Some(evaluation) = progress.pending.take() {
            rounds += 1;
            let limit = {
                let (def, _) = tx.parts();
                def.len() + 8
            };
            if rounds > limit {
                return Err(ExecutionError::Invariant("evaluations did not settle").into());
            }
            let (resolution, spawn) = self.resolve(tx, &evaluation).await?;
            effects.children.extend(spawn);
            progress = {
                let (def, state) = tx.parts();
                sagat_domain::execution::resolve(def, state, resolution, now)?
            };
            events.append(&mut progress.events);
        }
        tx.persist(&events, effects).await?;
        Ok(value)
    }

    async fn resolve<Tx: RunTx>(
        &self,
        tx: &mut Tx,
        evaluation: &Evaluation,
    ) -> Result<(Resolution, Option<ChildSpawn>), EngineError> {
        let depth = tx.info().depth;
        let parent = tx.info().run_id;
        let data = tx.data().await?;
        Ok(match evaluation {
            Evaluation::Condition { node, expression } => (
                Resolution::Condition {
                    node: node.clone(),
                    outcome: self
                        .evaluator
                        .condition(expression, data)
                        .map_err(log_failure(node)),
                },
                None,
            ),
            Evaluation::Switch { node, cases } => {
                let mut selection = Ok(sagat_domain::execution::SwitchSelection::Default);
                for case in cases {
                    match self.evaluator.condition(case.when(), data) {
                        Ok(true) => {
                            selection = Ok(sagat_domain::execution::SwitchSelection::Case(
                                case.id().clone(),
                            ));
                            break;
                        }
                        Ok(false) => {}
                        Err(error) => {
                            log_failure(node)(error);
                            selection = Err(());
                            break;
                        }
                    }
                }
                (
                    Resolution::Switch {
                        node: node.clone(),
                        selection,
                    },
                    None,
                )
            }
            Evaluation::Correlation { node, expression } => (
                Resolution::Correlation {
                    node: node.clone(),
                    key: self
                        .evaluator
                        .correlation_key(expression, data)
                        .map_err(log_failure(node)),
                },
                None,
            ),
            Evaluation::Child { node, input } => {
                let outcome = if depth >= MAX_CHILD_DEPTH {
                    Err(FailureCode::child_depth_exceeded())
                } else {
                    self.evaluator
                        .child_input(input.as_ref(), data)
                        .map_err(log_failure(node))
                        .map_err(|()| FailureCode::expression_error())
                        .and_then(|bytes| {
                            // An input beyond the inline limit would be refused when the child is
                            // created; refusing it here fails the step with a code instead.
                            if bytes.len() > MAX_PAYLOAD_BYTES {
                                Err(FailureCode::expression_error())
                            } else {
                                Ok(bytes)
                            }
                        })
                };
                match outcome {
                    Ok(input) => {
                        let child = child_run_id(parent, node);
                        (
                            Resolution::Child {
                                node: node.clone(),
                                child: Ok(child),
                            },
                            Some(ChildSpawn {
                                node: node.clone(),
                                child,
                                input,
                            }),
                        )
                    }
                    Err(code) => (
                        Resolution::Child {
                            node: node.clone(),
                            child: Err(code),
                        },
                        None,
                    ),
                }
            }
        })
    }
}

/// An expression that fails fails its node with `EXPRESSION_ERROR`; the cause is logged for
/// operators (it can mention field names, never values) and is not part of the journal.
fn log_failure(node: &NodeId) -> impl Fn(ExpressionFailure) + '_ {
    move |failure| {
        tracing::warn!(node = %node, error = %failure, "expression evaluation failed");
    }
}

/// Parses stored documents once and keeps the result: versions are immutable, so a parsed definition
/// never goes stale.
#[derive(Clone)]
pub struct DefinitionCache {
    codec: Arc<dyn IrCodec>,
    entries: Arc<std::sync::Mutex<CacheEntries>>,
    capacity: usize,
    /// Upper bound on the estimated memory of all cached definitions.
    max_bytes: usize,
}

#[derive(Default)]
struct CacheEntries {
    map: BTreeMap<WorkflowVersionId, (u64, Arc<WorkflowDefinition>)>,
    tick: u64,
    bytes: usize,
}

impl std::fmt::Debug for DefinitionCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DefinitionCache")
            .field("capacity", &self.capacity)
            .finish_non_exhaustive()
    }
}

impl DefinitionCache {
    /// A cache of at most `capacity` definitions and [`DEFAULT_CACHE_BYTES`] of estimated memory.
    #[must_use]
    pub fn new(codec: Arc<dyn IrCodec>, capacity: usize) -> Self {
        Self::with_budget(codec, capacity, DEFAULT_CACHE_BYTES)
    }

    /// A cache bounded both by count and by the estimated memory of what it holds, so a few huge
    /// definitions cannot crowd out memory the process needs. The entry just parsed is always kept,
    /// however large, so a definition bigger than the budget still works (it is simply not shared).
    #[must_use]
    pub fn with_budget(codec: Arc<dyn IrCodec>, capacity: usize, max_bytes: usize) -> Self {
        Self {
            codec,
            entries: Arc::default(),
            capacity: capacity.max(1),
            max_bytes,
        }
    }

    /// The parsed definition of `version` if it is cached.
    #[must_use]
    pub fn get(&self, version: WorkflowVersionId) -> Option<Arc<WorkflowDefinition>> {
        let mut guard = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.tick += 1;
        let tick = guard.tick;
        let (used, def) = guard.map.get_mut(&version)?;
        *used = tick;
        Some(Arc::clone(def))
    }

    /// The parsed definition of `version`, parsing `document` on a miss.
    ///
    /// # Errors
    /// [`IrError`] when the stored document no longer parses, which means it was changed behind the
    /// application's back.
    pub fn get_or_parse(
        &self,
        version: WorkflowVersionId,
        document: &[u8],
    ) -> Result<Arc<WorkflowDefinition>, IrError> {
        {
            let mut guard = self
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.tick += 1;
            let tick = guard.tick;
            if let Some((used, def)) = guard.map.get_mut(&version) {
                *used = tick;
                return Ok(Arc::clone(def));
            }
        }
        // Parse outside the lock: it can take milliseconds and other versions must not wait.
        let parsed = Arc::new(self.codec.parse(document)?.definition);
        let mut guard = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.tick += 1;
        let tick = guard.tick;
        // Two callers can parse the same version at once; the later one replaces the earlier.
        if let Some((_, previous)) = guard.map.remove(&version) {
            guard.bytes = guard.bytes.saturating_sub(previous.footprint());
        }
        guard.bytes += parsed.footprint();
        guard.map.insert(version, (tick, Arc::clone(&parsed)));
        // Evict the least recently used until both bounds hold; never the entry just inserted.
        while guard.map.len() > 1
            && (guard.map.len() > self.capacity || guard.bytes > self.max_bytes)
        {
            let oldest = guard
                .map
                .iter()
                .filter(|(k, _)| **k != version)
                .min_by_key(|(_, (used, _))| *used)
                .map(|(k, _)| *k);
            let Some(key) = oldest else { break };
            if let Some((_, gone)) = guard.map.remove(&key) {
                guard.bytes = guard.bytes.saturating_sub(gone.footprint());
            }
        }
        Ok(parsed)
    }
}
