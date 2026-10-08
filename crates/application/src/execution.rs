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
    CorrelationKey, Evaluation, ExecutionError, Progress, Resolution, RunEvent, RunId, RunState,
    RunStatus, StopReason,
};
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::workflow::{Expression, NodeId, WorkflowDefinition};
use time::OffsetDateTime;

use sagat_domain::audit::AuditRecord;

use crate::definitions::{Idempotency, IrCodec, IrError};
use crate::error::StoreError;

/// Largest payload (run input, step output) kept inline; bigger ones need a blob store, which is not
/// built yet (research.md, gap G19).
pub const MAX_PAYLOAD_BYTES: usize = 256 * 1024;

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

/// The data expressions can read.
#[derive(Debug, Clone, Default)]
pub struct RunData {
    pub run_id: Option<RunId>,
    pub input: Vec<u8>,
    /// Outputs of the steps that succeeded, by node id.
    pub outputs: BTreeMap<NodeId, Vec<u8>>,
}

/// An expression failed to evaluate. The detail is for logs; it never reaches clients.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("expression failed: {0}")]
pub struct ExpressionFailure(pub String);

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

    /// The JSON document a worker receives as the input of `node`.
    fn task_input(&self, data: &RunData, node: &NodeId) -> Vec<u8>;

    /// Whether `bytes` is a well-formed JSON document of any kind.
    fn is_json(&self, bytes: &[u8]) -> bool;

    /// Whether `bytes` is a well-formed JSON object (a run input must be one).
    fn is_json_object(&self, bytes: &[u8]) -> bool;
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
    pub deadline_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
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
        }
    }
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

    /// When the lease held on `node` expires (the renewal capped by the attempt deadline), or
    /// `None` when no lease is held.
    fn lease_expiry(
        &mut self,
        node: &NodeId,
    ) -> impl Future<Output = Result<Option<OffsetDateTime>, StoreError>> + Send;

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
        effects: Effects,
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
            let resolution = self.resolve(tx, &evaluation).await?;
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
    ) -> Result<Resolution, EngineError> {
        let data = tx.data().await?;
        Ok(match evaluation {
            Evaluation::Condition { node, expression } => Resolution::Condition {
                node: node.clone(),
                outcome: self
                    .evaluator
                    .condition(expression, data)
                    .map_err(log_failure(node)),
            },
            Evaluation::Correlation { node, expression } => Resolution::Correlation {
                node: node.clone(),
                key: self
                    .evaluator
                    .correlation_key(expression, data)
                    .map_err(log_failure(node)),
            },
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
}

#[derive(Default)]
struct CacheEntries {
    map: BTreeMap<WorkflowVersionId, (u64, Arc<WorkflowDefinition>)>,
    tick: u64,
}

impl std::fmt::Debug for DefinitionCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DefinitionCache")
            .field("capacity", &self.capacity)
            .finish_non_exhaustive()
    }
}

impl DefinitionCache {
    #[must_use]
    pub fn new(codec: Arc<dyn IrCodec>, capacity: usize) -> Self {
        Self {
            codec,
            entries: Arc::default(),
            capacity: capacity.max(1),
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
        guard.map.insert(version, (tick, Arc::clone(&parsed)));
        while guard.map.len() > self.capacity {
            let oldest = guard
                .map
                .iter()
                .min_by_key(|(_, (used, _))| *used)
                .map(|(k, _)| *k);
            match oldest {
                Some(key) => {
                    guard.map.remove(&key);
                }
                None => break,
            }
        }
        Ok(parsed)
    }
}
