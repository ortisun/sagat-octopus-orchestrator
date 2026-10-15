//! Transaction boundaries and authorized commands for bounded execution chains.
//!
//! Chain initialization is deliberately expressed as data that a run-creation transaction can
//! persist before it commits. Inspection, signalling and cancellation address the serialized
//! current head through the public control API. Creating a chain and moving its head remain
//! internal scheduler operations until canonical workflow routing is integrated.

use std::future::Future;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::execution::{
    self, ChainError, ChainFinalOutcome, ChainPolicy, ChainScope, ContinuationTransfer, Progress,
    RunId, RunStatus, StepStatus, StopReason, WaitingOn,
};
use sagat_domain::identity::{NamespaceId, Permission, Principal, Subject, TenantId};
use sagat_domain::workflow::{NodeId, NodeKind};
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

/// Chain metadata captured by the transaction that creates the initial run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainInitialization {
    scope: ChainScope,
    initial_run_id: RunId,
    policy: ChainPolicy,
}

impl ChainInitialization {
    #[must_use]
    pub const fn new(scope: ChainScope, initial_run_id: RunId, policy: ChainPolicy) -> Self {
        Self {
            scope,
            initial_run_id,
            policy,
        }
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

/// Scheduler-fenced application of one final child-chain outcome to its original parent step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildChainSettlementCommand {
    scope: ChainScope,
    parent_chain_id: RunId,
    expected_parent_head: RunId,
    expected_parent_chain_revision: u64,
    expected_parent_run_version: u64,
    child_chain_id: RunId,
    scheduler: SchedulerAuthority,
    occurred_at: OffsetDateTime,
    task_outbox_id: Uuid,
}

impl ChildChainSettlementCommand {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        scope: ChainScope,
        parent_chain_id: RunId,
        expected_parent_head: RunId,
        expected_parent_chain_revision: u64,
        expected_parent_run_version: u64,
        child_chain_id: RunId,
        scheduler: SchedulerAuthority,
        occurred_at: OffsetDateTime,
        task_outbox_id: Uuid,
    ) -> Self {
        Self {
            scope,
            parent_chain_id,
            expected_parent_head,
            expected_parent_chain_revision,
            expected_parent_run_version,
            child_chain_id,
            scheduler,
            occurred_at,
            task_outbox_id,
        }
    }

    #[must_use]
    pub const fn scope(&self) -> ChainScope {
        self.scope
    }

    #[must_use]
    pub const fn parent_chain_id(&self) -> RunId {
        self.parent_chain_id
    }

    #[must_use]
    pub const fn expected_parent_head(&self) -> RunId {
        self.expected_parent_head
    }

    #[must_use]
    pub const fn expected_parent_chain_revision(&self) -> u64 {
        self.expected_parent_chain_revision
    }

    #[must_use]
    pub const fn expected_parent_run_version(&self) -> u64 {
        self.expected_parent_run_version
    }

    #[must_use]
    pub const fn child_chain_id(&self) -> RunId {
        self.child_chain_id
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
    pub const fn task_outbox_id(&self) -> Uuid {
        self.task_outbox_id
    }
}

/// Durable result of applying or replaying a child-chain outcome at the parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChildChainSettlementWrite {
    parent_run_id: RunId,
    child_chain_id: RunId,
    replayed: bool,
}

impl ChildChainSettlementWrite {
    #[must_use]
    pub const fn new(parent_run_id: RunId, child_chain_id: RunId, replayed: bool) -> Self {
        Self {
            parent_run_id,
            child_chain_id,
            replayed,
        }
    }

    #[must_use]
    pub const fn parent_run_id(self) -> RunId {
        self.parent_run_id
    }

    #[must_use]
    pub const fn child_chain_id(self) -> RunId {
        self.child_chain_id
    }

    #[must_use]
    pub const fn replayed(self) -> bool {
        self.replayed
    }
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

/// Internal continuation boundary. No public transport calls this port while runtime gates remain.
pub trait ChainProgressStore: Send + Sync {
    fn continue_as_new(
        &self,
        command: &ChainHandoffCommand,
    ) -> impl Future<Output = Result<ChainHandoffWrite, ChainProgressError>> + Send;

    fn finalize_chain(
        &self,
        command: &ChainFinalizationCommand,
    ) -> impl Future<Output = Result<ChainFinalizationWrite, ChainProgressError>> + Send;

    fn settle_child_chain(
        &self,
        command: &ChildChainSettlementCommand,
    ) -> impl Future<Output = Result<ChildChainSettlementWrite, ChainProgressError>> + Send;
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
        let retained: usize = tx.data().await?.outputs.values().map(Vec::len).sum();
        if retained.saturating_add(payload.len().saturating_mul(targets.len()))
            > crate::execution::MAX_RUN_OUTPUT_BYTES
        {
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
            if let Err(error) = self.store.record_denial(&audit).await {
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
