//! Transaction boundaries and authorized commands for bounded execution chains.
//!
//! Chain initialization is expressed as data that the run-creation transaction persists before it
//! commits. Inspection, signalling and cancellation address the serialized current head through
//! the public control API. The scheduler alone moves that head after the canonical planner reaches
//! a continuation boundary.

use std::future::Future;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::execution::{
    self, ChainError, ChainFinalOutcome, ChainPolicy, ChainScope, ContinuationTransfer,
    CorrelationKey, Progress, RunId, RunStatus, StepStatus, StopReason, WaitingOn,
};
use sagat_domain::identity::{NamespaceId, Permission, Principal, Subject, TenantId};
use sagat_domain::workflow::{Digest, NodeId, NodeKind};
use sha2::{Digest as _, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::clock::Clock;
use crate::definitions::{Idempotency, fingerprint};
use crate::error::{AppError, StoreError};
use crate::execution::{
    Actor, Effects, Engine, EngineError, ExpressionEvaluator, MAX_PAYLOAD_BYTES, RunTx,
};
use crate::identity::RequestContext;
use crate::primitives::SchedulerAuthority;
use crate::runs::RunStore;

/// The parent step a child chain answers to. The link names the parent run and its `child` node;
/// the store resolves the step identity inside the creating transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainParent {
    run: RunId,
    node: NodeId,
}

impl ChainParent {
    #[must_use]
    pub const fn new(run: RunId, node: NodeId) -> Self {
        Self { run, node }
    }

    #[must_use]
    pub const fn run(&self) -> RunId {
        self.run
    }

    #[must_use]
    pub const fn node(&self) -> &NodeId {
        &self.node
    }
}

/// Chain metadata captured by the transaction that creates the initial run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainInitialization {
    scope: ChainScope,
    initial_run_id: RunId,
    policy: ChainPolicy,
    parent: Option<ChainParent>,
}

impl ChainInitialization {
    #[must_use]
    pub const fn new(scope: ChainScope, initial_run_id: RunId, policy: ChainPolicy) -> Self {
        Self {
            scope,
            initial_run_id,
            policy,
            parent: None,
        }
    }

    /// Makes the chain a child of `parent`: the parent step waits for the chain's final outcome,
    /// never for one of its segments.
    #[must_use]
    pub fn with_parent(mut self, parent: ChainParent) -> Self {
        self.parent = Some(parent);
        self
    }

    #[must_use]
    pub const fn parent(&self) -> Option<&ChainParent> {
        self.parent.as_ref()
    }

    #[must_use]
    pub const fn scope(&self) -> ChainScope {
        self.scope
    }

    #[must_use]
    pub const fn initial_run_id(&self) -> RunId {
        self.initial_run_id
    }

    #[must_use]
    pub const fn policy(&self) -> ChainPolicy {
        self.policy
    }
}

/// Every caller-controlled value used by one serialized chain handoff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainHandoffCommand {
    scope: ChainScope,
    chain_id: RunId,
    expected_head: RunId,
    expected_chain_revision: u64,
    expected_run_version: u64,
    successor: RunId,
    transfer: ContinuationTransfer,
    scheduler: SchedulerAuthority,
    actor: Subject,
    trace_id: Option<String>,
    occurred_at: OffsetDateTime,
    audit: AuditRecord,
    continuation_outbox_id: Uuid,
    successor_outbox_id: Uuid,
}

impl ChainHandoffCommand {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        scope: ChainScope,
        chain_id: RunId,
        expected_head: RunId,
        expected_chain_revision: u64,
        expected_run_version: u64,
        successor: RunId,
        transfer: ContinuationTransfer,
        scheduler: SchedulerAuthority,
        actor: Subject,
        trace_id: Option<String>,
        occurred_at: OffsetDateTime,
        audit: AuditRecord,
        continuation_outbox_id: Uuid,
        successor_outbox_id: Uuid,
    ) -> Self {
        Self {
            scope,
            chain_id,
            expected_head,
            expected_chain_revision,
            expected_run_version,
            successor,
            transfer,
            scheduler,
            actor,
            trace_id,
            occurred_at,
            audit,
            continuation_outbox_id,
            successor_outbox_id,
        }
    }

    #[must_use]
    pub const fn scope(&self) -> ChainScope {
        self.scope
    }

    #[must_use]
    pub const fn chain_id(&self) -> RunId {
        self.chain_id
    }

    #[must_use]
    pub const fn expected_head(&self) -> RunId {
        self.expected_head
    }

    #[must_use]
    pub const fn expected_chain_revision(&self) -> u64 {
        self.expected_chain_revision
    }

    #[must_use]
    pub const fn expected_run_version(&self) -> u64 {
        self.expected_run_version
    }

    #[must_use]
    pub const fn successor(&self) -> RunId {
        self.successor
    }

    #[must_use]
    pub const fn transfer(&self) -> &ContinuationTransfer {
        &self.transfer
    }

    #[must_use]
    pub const fn scheduler(&self) -> &SchedulerAuthority {
        &self.scheduler
    }

    #[must_use]
    pub const fn actor(&self) -> &Subject {
        &self.actor
    }

    #[must_use]
    pub fn trace_id(&self) -> Option<&str> {
        self.trace_id.as_deref()
    }

    #[must_use]
    pub const fn occurred_at(&self) -> OffsetDateTime {
        self.occurred_at
    }

    #[must_use]
    pub const fn audit(&self) -> &AuditRecord {
        &self.audit
    }

    #[must_use]
    pub const fn continuation_outbox_id(&self) -> Uuid {
        self.continuation_outbox_id
    }

    #[must_use]
    pub const fn successor_outbox_id(&self) -> Uuid {
        self.successor_outbox_id
    }
}

/// Durable result of applying or replaying a handoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainHandoffWrite {
    predecessor: RunId,
    successor: RunId,
    successor_index: u16,
    replayed: bool,
}

/// Evaluated transfer values tied to the immutable run snapshot that selected them.
///
/// The write adapter rechecks these bytes, their digests and the snapshot revisions after taking
/// the chain and run locks. Evaluation is pure and may happen before the write transaction; no
/// mutation may commit unless those compare-and-swap guards still match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluatedChainHandoffCommand {
    handoff: ChainHandoffCommand,
    boundary: NodeId,
    input: Vec<u8>,
    state: Option<Vec<u8>>,
    expected_state_revision: Option<u64>,
    correlation: Option<CorrelationKey>,
}

/// Bounded values selected from one immutable run snapshot before the handoff lock is reacquired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContinuationSelection {
    boundary: NodeId,
    input: Vec<u8>,
    state: Option<Vec<u8>>,
    expected_state_revision: Option<u64>,
    correlation: Option<CorrelationKey>,
}

impl ContinuationSelection {
    /// # Errors
    /// Returns a typed preparation error when a selected value exceeds the retained payload limit
    /// or state presence is not pinned to a revision.
    pub fn new(
        boundary: NodeId,
        input: Vec<u8>,
        state: Option<Vec<u8>>,
        expected_state_revision: Option<u64>,
    ) -> Result<Self, ContinuationPreparationError> {
        if input.len() > MAX_PAYLOAD_BYTES
            || state
                .as_ref()
                .is_some_and(|value| value.len() > MAX_PAYLOAD_BYTES)
        {
            return Err(ContinuationPreparationError::PayloadTooLarge);
        }
        if state.is_some() != expected_state_revision.is_some() {
            return Err(ContinuationPreparationError::StateRevisionMismatch);
        }
        Ok(Self {
            boundary,
            input,
            state,
            expected_state_revision,
            correlation: None,
        })
    }

    /// Selects the correlation key the successor's entry `waitEvent` waits on, evaluated over the
    /// transferred input. The successor is then created already waiting (G372): the key, the wait
    /// deadline and the event subscription commit with the hand-off. A selection without a key is
    /// for a successor that starts with an activity; the write refuses a selection that does not
    /// match the entry the definition declares.
    #[must_use]
    pub fn with_correlation_key(mut self, key: CorrelationKey) -> Self {
        self.correlation = Some(key);
        self
    }

    #[must_use]
    pub const fn correlation_key(&self) -> Option<&CorrelationKey> {
        self.correlation.as_ref()
    }

    #[must_use]
    pub fn transfer(&self) -> ContinuationTransfer {
        ContinuationTransfer::new(digest(&self.input), self.state.as_deref().map(digest))
    }

    /// Binds this selection to the command whose CAS guards describe the same snapshot.
    ///
    /// # Errors
    /// Returns [`ContinuationPreparationError::DigestMismatch`] if the command carries a different
    /// transfer identity.
    pub fn into_command(
        self,
        handoff: ChainHandoffCommand,
    ) -> Result<EvaluatedChainHandoffCommand, ContinuationPreparationError> {
        if self.transfer() != *handoff.transfer() {
            return Err(ContinuationPreparationError::DigestMismatch);
        }
        Ok(EvaluatedChainHandoffCommand {
            handoff,
            boundary: self.boundary,
            input: self.input,
            state: self.state,
            expected_state_revision: self.expected_state_revision,
            correlation: self.correlation,
        })
    }
}

impl EvaluatedChainHandoffCommand {
    /// Binds evaluated bytes to the digests already present in the handoff command.
    ///
    /// # Errors
    /// Returns a typed preparation error when a value exceeds the workflow payload ceiling, its
    /// digest differs from the selected transfer, or state presence does not match its revision.
    pub fn new(
        handoff: ChainHandoffCommand,
        boundary: NodeId,
        input: Vec<u8>,
        state: Option<Vec<u8>>,
        expected_state_revision: Option<u64>,
    ) -> Result<Self, ContinuationPreparationError> {
        ContinuationSelection::new(boundary, input, state, expected_state_revision)?
            .into_command(handoff)
    }

    #[must_use]
    pub const fn handoff(&self) -> &ChainHandoffCommand {
        &self.handoff
    }

    #[must_use]
    pub const fn boundary(&self) -> &NodeId {
        &self.boundary
    }

    #[must_use]
    pub fn input(&self) -> &[u8] {
        &self.input
    }

    #[must_use]
    pub fn state(&self) -> Option<&[u8]> {
        self.state.as_deref()
    }

    #[must_use]
    pub const fn expected_state_revision(&self) -> Option<u64> {
        self.expected_state_revision
    }

    /// The key the successor's entry wait subscribes with; `None` for an activity entrypoint.
    #[must_use]
    pub const fn correlation_key(&self) -> Option<&CorrelationKey> {
        self.correlation.as_ref()
    }
}

fn digest(value: &[u8]) -> Digest {
    Digest::new(Sha256::digest(value).into())
}

/// A continuation selection that cannot safely enter the write transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ContinuationPreparationError {
    #[error("a continuation transfer exceeds the retained payload limit")]
    PayloadTooLarge,
    #[error("continuation transfer bytes do not match their selected digests")]
    DigestMismatch,
    #[error("continuation state and its expected revision must be present together")]
    StateRevisionMismatch,
}

/// Serialized terminal decision for the current chain head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainFinalizationCommand {
    scope: ChainScope,
    chain_id: RunId,
    expected_head: RunId,
    expected_chain_revision: u64,
    expected_run_version: u64,
    status: RunStatus,
    scheduler: SchedulerAuthority,
    occurred_at: OffsetDateTime,
    audit: AuditRecord,
    run_outbox_id: Uuid,
    chain_outbox_id: Uuid,
}

impl ChainFinalizationCommand {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        scope: ChainScope,
        chain_id: RunId,
        expected_head: RunId,
        expected_chain_revision: u64,
        expected_run_version: u64,
        status: RunStatus,
        scheduler: SchedulerAuthority,
        occurred_at: OffsetDateTime,
        audit: AuditRecord,
        run_outbox_id: Uuid,
        chain_outbox_id: Uuid,
    ) -> Self {
        Self {
            scope,
            chain_id,
            expected_head,
            expected_chain_revision,
            expected_run_version,
            status,
            scheduler,
            occurred_at,
            audit,
            run_outbox_id,
            chain_outbox_id,
        }
    }

    #[must_use]
    pub const fn scope(&self) -> ChainScope {
        self.scope
    }

    #[must_use]
    pub const fn chain_id(&self) -> RunId {
        self.chain_id
    }

    #[must_use]
    pub const fn expected_head(&self) -> RunId {
        self.expected_head
    }

    #[must_use]
    pub const fn expected_chain_revision(&self) -> u64 {
        self.expected_chain_revision
    }

    #[must_use]
    pub const fn expected_run_version(&self) -> u64 {
        self.expected_run_version
    }

    #[must_use]
    pub const fn status(&self) -> RunStatus {
        self.status
    }

    #[must_use]
    pub const fn scheduler(&self) -> &SchedulerAuthority {
        &self.scheduler
    }

    #[must_use]
    pub const fn occurred_at(&self) -> OffsetDateTime {
        self.occurred_at
    }

    #[must_use]
    pub const fn audit(&self) -> &AuditRecord {
        &self.audit
    }

    #[must_use]
    pub const fn run_outbox_id(&self) -> Uuid {
        self.run_outbox_id
    }

    #[must_use]
    pub const fn chain_outbox_id(&self) -> Uuid {
        self.chain_outbox_id
    }
}

/// Durable result of applying or replaying a terminal chain outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainFinalizationWrite {
    head: RunId,
    outcome: ChainFinalOutcome,
    replayed: bool,
}

impl ChainFinalizationWrite {
    #[must_use]
    pub const fn new(head: RunId, outcome: ChainFinalOutcome, replayed: bool) -> Self {
        Self {
            head,
            outcome,
            replayed,
        }
    }

    #[must_use]
    pub const fn head(self) -> RunId {
        self.head
    }

    #[must_use]
    pub const fn outcome(self) -> ChainFinalOutcome {
        self.outcome
    }

    #[must_use]
    pub const fn replayed(self) -> bool {
        self.replayed
    }
}

impl ChainHandoffWrite {
    #[must_use]
    pub const fn new(
        predecessor: RunId,
        successor: RunId,
        successor_index: u16,
        replayed: bool,
    ) -> Self {
        Self {
            predecessor,
            successor,
            successor_index,
            replayed,
        }
    }

    #[must_use]
    pub const fn predecessor(self) -> RunId {
        self.predecessor
    }

    #[must_use]
    pub const fn successor(self) -> RunId {
        self.successor
    }

    #[must_use]
    pub const fn successor_index(self) -> u16 {
        self.successor_index
    }

    #[must_use]
    pub const fn replayed(self) -> bool {
        self.replayed
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChainProgressError {
    #[error("execution chain or current run was not found")]
    Missing,
    #[error("the chain projection is inconsistent with its immutable lineage")]
    CorruptProjection,
    #[error("the continuation audit evidence does not match the handoff")]
    AuditMismatch,
    #[error("scheduler ownership changed before the handoff committed")]
    SchedulerFenceLost,
    #[error("the successor input or state is not retained by this tenant")]
    PayloadMissing,
    #[error("active-run capacity is currently full")]
    AtCapacity,
    #[error("the requested chain transition is not integrated with the canonical runtime")]
    RuntimeNotIntegrated,
    #[error("continuation rejected: {0}")]
    Chain(#[from] ChainError),
    #[error(transparent)]
    Storage(#[from] StoreError),
}

/// Scheduler-owned continuation boundary; public transports never write a handoff directly.
pub trait ChainProgressStore: Send + Sync {
    fn continue_as_new(
        &self,
        command: &ChainHandoffCommand,
    ) -> impl Future<Output = Result<ChainHandoffWrite, ChainProgressError>> + Send;

    /// Commits a prepared continuation only if its chain, run and declared-state snapshot still
    /// match under the canonical locks.
    fn continue_evaluated(
        &self,
        _command: &EvaluatedChainHandoffCommand,
    ) -> impl Future<Output = Result<ChainHandoffWrite, ChainProgressError>> + Send {
        async { Err(ChainProgressError::RuntimeNotIntegrated) }
    }

    fn finalize_chain(
        &self,
        command: &ChainFinalizationCommand,
    ) -> impl Future<Output = Result<ChainFinalizationWrite, ChainProgressError>> + Send;
}

/// Chain row held before its current run is locked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockedChainHead {
    scope: ChainScope,
    chain_id: RunId,
    head: RunId,
    revision: u64,
    cancel_requested: bool,
    final_outcome: Option<ChainFinalOutcome>,
}

impl LockedChainHead {
    #[must_use]
    pub const fn restore(
        scope: ChainScope,
        chain_id: RunId,
        head: RunId,
        revision: u64,
        cancel_requested: bool,
        final_outcome: Option<ChainFinalOutcome>,
    ) -> Self {
        Self {
            scope,
            chain_id,
            head,
            revision,
            cancel_requested,
            final_outcome,
        }
    }

    #[must_use]
    pub const fn scope(self) -> ChainScope {
        self.scope
    }

    #[must_use]
    pub const fn chain_id(self) -> RunId {
        self.chain_id
    }

    #[must_use]
    pub const fn head(self) -> RunId {
        self.head
    }

    #[must_use]
    pub const fn revision(self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn cancel_requested(self) -> bool {
        self.cancel_requested
    }

    #[must_use]
    pub const fn final_outcome(self) -> Option<ChainFinalOutcome> {
        self.final_outcome
    }
}

/// Immutable lineage entry returned by chain inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainSegmentView {
    pub index: u16,
    pub run_id: RunId,
    pub predecessor: Option<RunId>,
    pub successor: Option<RunId>,
}

/// Tenant-scoped chain view read from one database snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainInspection {
    pub chain_id: RunId,
    pub current_head: RunId,
    pub revision: u64,
    pub cancel_requested: bool,
    pub final_outcome: Option<ChainFinalOutcome>,
    pub segments: Vec<ChainSegmentView>,
}

/// Most segments one page of a chain's runs may carry (the list convention of the API).
pub const MAX_CHAIN_RUN_PAGE: u32 = 200;

/// One segment of a chain as a list item: lineage and lifecycle, never a payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainRunItem {
    pub index: u16,
    pub run_id: RunId,
    pub status: RunStatus,
    pub completion_kind: Option<execution::SegmentCompletionKind>,
    pub predecessor: Option<RunId>,
    pub successor: Option<RunId>,
    pub created_at: OffsetDateTime,
    pub finished_at: Option<OffsetDateTime>,
}

/// One page of a chain's runs in segment order. `next_after` is the last index on a full page that
/// has more behind it; the API turns it into an opaque cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainRunPage {
    pub items: Vec<ChainRunItem>,
    pub next_after: Option<u16>,
}

/// A run transaction opened only after its chain row was serialized.
pub trait ChainRunTx: RunTx {
    fn locked_chain(&self) -> Result<&LockedChainHead, StoreError>;
}

/// Storage needed by the authorized inspect, signal and cancel commands.
pub trait ChainCommandStore: RunStore
where
    Self::Tx: ChainRunTx,
{
    fn begin_chain_head(
        &self,
        scope: ChainScope,
        chain_id: RunId,
    ) -> impl Future<Output = Result<Option<Self::Tx>, StoreError>> + Send;

    fn inspect_chain(
        &self,
        scope: ChainScope,
        chain_id: RunId,
    ) -> impl Future<Output = Result<Option<ChainInspection>, StoreError>> + Send;

    fn chain_exists_outside(
        &self,
        tenant_id: TenantId,
        chain_id: RunId,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// The segments after index `after` (all of them from the first when `None`), at most `limit`,
    /// in segment order, or `None` when the chain does not exist in this tenant and namespace.
    fn list_chain_runs(
        &self,
        scope: ChainScope,
        chain_id: RunId,
        after: Option<u16>,
        limit: u32,
    ) -> impl Future<Output = Result<Option<ChainRunPage>, StoreError>> + Send;
}

/// Canonical private command path for the current head of an execution chain.
pub struct ChainCommandService<S, C, E> {
    store: S,
    clock: C,
    engine: Engine<E>,
}

impl<S, C, E> ChainCommandService<S, C, E>
where
    S: ChainCommandStore,
    S::Tx: ChainRunTx,
    C: Clock,
    E: ExpressionEvaluator,
{
    #[must_use]
    pub const fn new(store: S, clock: C, engine: Engine<E>) -> Self {
        Self {
            store,
            clock,
            engine,
        }
    }

    /// Reads the lineage after authorizing the caller and applying tenant and namespace filters.
    pub async fn inspect(
        &self,
        principal: &Principal,
        namespace_id: NamespaceId,
        chain_id: RunId,
        ctx: &RequestContext,
    ) -> Result<ChainInspection, AppError> {
        principal.authorize(Permission::RunRead)?;
        let scope = ChainScope::new(principal.tenant_id(), namespace_id);
        match self.store.inspect_chain(scope, chain_id).await? {
            Some(inspection) => Ok(inspection),
            None => Err(self.missing_async(principal, chain_id, ctx).await),
        }
    }

    /// One bounded page of the chain's runs in segment order. Authorization, tenant opacity and the
    /// denial audit are those of [`Self::inspect`]; the segment index is a stable cursor because a
    /// segment is only ever appended.
    pub async fn list_runs(
        &self,
        principal: &Principal,
        namespace_id: NamespaceId,
        chain_id: RunId,
        after: Option<u16>,
        limit: u32,
        ctx: &RequestContext,
    ) -> Result<ChainRunPage, AppError> {
        principal.authorize(Permission::RunRead)?;
        let scope = ChainScope::new(principal.tenant_id(), namespace_id);
        let limit = limit.clamp(1, MAX_CHAIN_RUN_PAGE);
        match self
            .store
            .list_chain_runs(scope, chain_id, after, limit)
            .await?
        {
            Some(page) => Ok(page),
            None => Err(self.missing_async(principal, chain_id, ctx).await),
        }
    }

    /// Delivers a signal only to waits that already exist on the serialized current head.
    #[allow(clippy::too_many_arguments)]
    pub async fn signal(
        &self,
        principal: &Principal,
        namespace_id: NamespaceId,
        chain_id: RunId,
        expected_head: Option<RunId>,
        name: &str,
        payload: &[u8],
        key: &str,
        ctx: &RequestContext,
    ) -> Result<Vec<NodeId>, AppError> {
        principal.authorize(Permission::RunSignal)?;
        if payload.len() > MAX_PAYLOAD_BYTES || !self.engine.evaluator().is_json(payload) {
            return Err(AppError::NotAllowed(
                "the signal payload must be JSON of at most 256 KiB",
            ));
        }
        let expected = expected_head.map_or([0_u8; 16], |run| *run.as_uuid().as_bytes());
        let idempotency = Idempotency {
            scope: format!("chain.signal:{chain_id}"),
            key: key.to_owned(),
            fingerprint: fingerprint(&[&expected, name.as_bytes(), payload]),
        };
        let scope = ChainScope::new(principal.tenant_id(), namespace_id);
        let Some(mut tx) = self.store.begin_chain_head(scope, chain_id).await? else {
            return Err(self.missing_async(principal, chain_id, ctx).await);
        };
        if let Some(delivered) = tx.replay(&idempotency).await? {
            return Ok(parse_nodes(&delivered));
        }
        let chain = *tx.locked_chain()?;
        ensure_open_head(chain, expected_head)?;
        let targets: Vec<NodeId> = {
            let (definition, state) = tx.parts();
            state
                .steps()
                .filter(|(_, step)| {
                    step.status() == StepStatus::Waiting
                        && matches!(step.waiting_on(), Some(WaitingOn::Event { .. }))
                })
                .filter(|(node, _)| {
                    matches!(
                        definition.node(node).map(sagat_domain::workflow::Node::kind),
                        Some(NodeKind::WaitEvent { event_type, .. }) if event_type.as_str() == name
                    )
                })
                .map(|(node, _)| node.clone())
                .collect()
        };
        if targets.is_empty() {
            return Err(AppError::StateConflict(
                "the chain head is not waiting for that signal",
            ));
        }
        let retained = tx.logical_output_bytes().await?;
        let payload_bytes = u64::try_from(payload.len()).unwrap_or(u64::MAX);
        let copies = u64::try_from(targets.len()).unwrap_or(u64::MAX);
        let additional = payload_bytes.saturating_mul(copies);
        if retained.saturating_add(additional) > crate::execution::MAX_RUN_OUTPUT_BYTES as u64 {
            return Err(AppError::QuotaExceeded("output retained by one run"));
        }
        let now = self.clock.now();
        let mut effects = Effects::by(
            Actor::Principal(principal.subject().as_str().to_owned()),
            ctx.trace_id.clone(),
        );
        effects.audit = Some(
            AuditRecord::by(
                principal,
                AuditAction::RunSignalled,
                ResourceType::Run,
                chain.head().to_string(),
                ctx.trace_id.clone(),
                now,
            )
            .with_reason(format!(
                "chain signal '{}'",
                name.chars().take(64).collect::<String>()
            )),
        );
        effects.outputs = targets
            .iter()
            .map(|node| (node.clone(), payload.to_vec()))
            .collect();
        effects.idempotency = Some((idempotency, join_nodes(&targets)));
        let woken = targets.clone();
        self.engine
            .run(&mut tx, effects, move |definition, state, now| {
                let mut events = Vec::new();
                let mut pending = None;
                for node in &targets {
                    let mut progress = execution::satisfy_wait(definition, state, node, now)?;
                    events.append(&mut progress.events);
                    pending = pending.or(progress.pending);
                }
                Ok(((), Progress { events, pending }))
            })
            .await
            .map_err(chain_engine_error)?;
        tx.commit().await?;
        Ok(woken)
    }

    /// Latches cancellation on the chain and stops its current head in the same transaction.
    #[allow(clippy::too_many_arguments)]
    pub async fn cancel(
        &self,
        principal: &Principal,
        namespace_id: NamespaceId,
        chain_id: RunId,
        expected_head: Option<RunId>,
        reason: &str,
        key: &str,
        ctx: &RequestContext,
    ) -> Result<(), AppError> {
        principal.authorize(Permission::RunOperate)?;
        let expected = expected_head.map_or([0_u8; 16], |run| *run.as_uuid().as_bytes());
        let idempotency = Idempotency {
            scope: format!("chain.cancel:{chain_id}"),
            key: key.to_owned(),
            fingerprint: fingerprint(&[&expected, reason.as_bytes()]),
        };
        let scope = ChainScope::new(principal.tenant_id(), namespace_id);
        let Some(mut tx) = self.store.begin_chain_head(scope, chain_id).await? else {
            return Err(self.missing_async(principal, chain_id, ctx).await);
        };
        if tx.replay(&idempotency).await?.is_some() {
            return Ok(());
        }
        let chain = *tx.locked_chain()?;
        ensure_open_head(chain, expected_head)?;
        tx.latch_chain_cancel().await?;
        let now = self.clock.now();
        let mut effects = Effects::by(
            Actor::Principal(principal.subject().as_str().to_owned()),
            ctx.trace_id.clone(),
        );
        effects.audit = Some(
            AuditRecord::by(
                principal,
                AuditAction::RunCancelRequested,
                ResourceType::Run,
                chain.head().to_string(),
                ctx.trace_id.clone(),
                now,
            )
            .with_reason(reason.chars().take(512).collect::<String>()),
        );
        effects.idempotency = Some((idempotency, chain.head().to_string()));
        self.engine
            .run(&mut tx, effects, |definition, state, now| {
                let progress =
                    execution::stop_run(definition, state, StopReason::Cancelled, None, now)?;
                Ok(((), progress))
            })
            .await
            .map_err(chain_engine_error)?;
        tx.commit().await?;
        Ok(())
    }

    async fn missing_async(
        &self,
        principal: &Principal,
        chain_id: RunId,
        ctx: &RequestContext,
    ) -> AppError {
        if let Ok(true) = self
            .store
            .chain_exists_outside(principal.tenant_id(), chain_id)
            .await
        {
            let audit = AuditRecord::by(
                principal,
                AuditAction::CrossTenantAccessDenied,
                ResourceType::Run,
                chain_id.to_string(),
                ctx.trace_id.clone(),
                self.clock.now(),
            )
            .with_reason("resource not accessible from this tenant");
            if let Err(error) = RunStore::record_denial(&self.store, &audit).await {
                tracing::error!(%error, action = %audit.action(), "failed to record chain denial");
            }
        }
        AppError::NotFound
    }
}

fn ensure_open_head(chain: LockedChainHead, expected_head: Option<RunId>) -> Result<(), AppError> {
    if expected_head.is_some_and(|expected| expected != chain.head()) {
        return Err(AppError::StateConflict("the chain head changed"));
    }
    if chain.final_outcome().is_some() {
        return Err(AppError::StateConflict("the chain has already finished"));
    }
    if chain.cancel_requested() {
        return Err(AppError::StateConflict(
            "chain cancellation was already requested",
        ));
    }
    Ok(())
}

fn join_nodes(nodes: &[NodeId]) -> String {
    nodes
        .iter()
        .map(NodeId::as_str)
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_nodes(value: &str) -> Vec<NodeId> {
    value
        .split(',')
        .filter(|node| !node.is_empty())
        .filter_map(|node| NodeId::parse(node).ok())
        .collect()
}

fn chain_engine_error(error: EngineError) -> AppError {
    match error {
        EngineError::Store(error) => error.into(),
        EngineError::Execution(error) => crate::runs::execution_error(&error),
    }
}
