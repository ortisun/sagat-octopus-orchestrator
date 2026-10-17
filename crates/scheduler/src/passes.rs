use std::future::Future;
use std::hash::Hash;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use futures_util::stream;
use sagat_application::approval_scheduler::{
    ApprovalAdvance, ApprovalAdvanceError, ApprovalScheduler, ApprovalSchedulerStore, DueApproval,
};
use sagat_application::chain_progress::{
    ChainCommandStore, ChainHandoffCommand, ChainHandoffWrite, ChainProgressError,
    ChainProgressStore, ChainRunTx, ContinuationPreparationError, ContinuationSelection,
    LockedChainHead,
};
use sagat_application::definitions::{Applied, DefinitionStore, Idempotency};
use sagat_application::entropy::Entropy;
use sagat_application::error::StoreError;
use sagat_application::execution::{
    Actor, DefinitionCache, Effects, Engine, EngineError, ExpressionEvaluator, MAX_PAYLOAD_BYTES,
    MAX_RUN_OUTPUT_BYTES, ParentLink, RunInfo, RunTx, ScopeInput,
};
use sagat_application::loop_progress::{
    LoopCandidate, LoopCheckpointCommand, LoopDelayElapsedCommand, LoopDrainCommand,
    LoopDrainFinishCommand, LoopEntryCommand, LoopInputError, LoopIterationDrainCommand,
    LoopNextActivity, LoopProgressError, LoopProgressStore, LoopTransition,
};
use sagat_application::maintenance::{
    EndedChild, LeaseLoss, MaintenanceStore, PendingChild, PendingContinuation, RunRef, StepRef,
    Window,
};
use sagat_application::primitives::{
    CollectionAdmission, CollectionCheckpoint, CollectionDrain, CollectionDrainFinish,
    CollectionEntry, CollectionFinish, CollectionIterationDrain, CollectionLimits,
    CollectionScopeId, CollectionStore, CollectionTransition, PrimitiveInputError,
    PrimitiveStoreError, SchedulerAuthority,
};
use sagat_application::quotas::RunLimits;
use sagat_application::runs::{Lock, NewRun, RunStore};
use sagat_application::shards::Lease;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::execution::RunId;
use sagat_domain::execution::{
    self, AddressedProgress, ChildOutcome, ExecutionError, ExecutionInstanceId, Expiry,
    FailureCode, LoopContinuation, LoopKind, LoopPhase, Progress, RunStatus, StopReason,
};
use sagat_domain::identity::{NamespaceId, Subject, TenantId};
use sagat_domain::sharding::{Shard, ShardSet};
use sagat_domain::workflow::{NodeId, NodeKind, WorkflowDefinition};
use sagat_telemetry::{
    WorkflowPrimitiveKind, WorkflowPrimitiveMetric, WorkflowPrimitiveRejectionCode,
};
use time::OffsetDateTime;
use tracing::Instrument;

use crate::fairness::{self, Weights};
use crate::shards::{Placement, Snapshot};

/// Delivered outbox messages deleted per housekeeping round.
const PURGE_BATCH: u32 = 5_000;
/// Batches one housekeeping call may purge (about 100 000 messages).
const PURGE_ROUNDS: u32 = 20;

/// Tunable limits of the passes.
#[derive(Debug, Clone, Copy)]
pub struct SchedulerSettings {
    /// Candidates examined per pass per tick.
    pub batch: u32,
    /// The least a pass rests its search for other tenants after a search that found nobody new (zero:
    /// never rests). Finding out that the tenants a pass left out are the only ones with work costs a
    /// read of their whole backlog; with one tenant's burst of a hundred thousand timers that is
    /// every tick for as long as it drains. The rest is this or ten times what the search took,
    /// whichever is longer (`fairness::rest`). During the rest the pass shares the first page of the
    /// oldest candidates, so a tenant that arrives behind a burst is found when the rest is over.
    pub discovery_rest: Duration,
    /// Whether a pass takes its candidates fairly between tenants (the default). Off, it takes the
    /// oldest `batch` of the shards it holds, as before feature 010: a diagnostic, to measure what
    /// fairness does (`tests/performance/scalability-ha`), not a mode to run in.
    pub fair: bool,
    /// Candidates of one pass that are acted on at the same time (each in its own transaction).
    /// One at a time costs a round trip of latency per candidate; this is what lets a replica
    /// fire the timers of a burst at the rate the database allows instead (`FR-010-002`).
    pub concurrency: usize,
    /// A `READY` step with no notification for this long gets a fresh one.
    pub rehint_after: Duration,
    /// Idempotency records older than this are purged.
    pub idempotency_retention: Duration,
    /// Delivered outbox messages older than this are purged.
    pub outbox_retention: Duration,
}

impl Default for SchedulerSettings {
    fn default() -> Self {
        Self {
            batch: 200,
            fair: true,
            discovery_rest: Duration::from_millis(100),
            concurrency: 8,
            rehint_after: Duration::from_secs(30),
            idempotency_retention: Duration::from_secs(7 * 24 * 3600),
            outbox_retention: Duration::from_secs(24 * 3600),
        }
    }
}

/// What one tick did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    pub collections_advanced: u32,
    pub loops_advanced: u32,
    pub continuations_advanced: u32,
    pub approvals_opened: u32,
    pub approvals_expired: u32,
    pub approval_routes_released: u32,
    pub stopped_runs_settled: u32,
    pub timers_fired: u32,
    pub waits_timed_out: u32,
    pub retries_released: u32,
    pub leases_expired: u32,
    pub deadlines_enforced: u32,
    pub hints_renewed: u32,
    pub children_started: u32,
    pub children_finished: u32,
    pub children_cancelled: u32,
    /// Runs stopped because an automatic transition could not fit retained content.
    pub quota_stops: u32,
    /// Candidates another scheduler or a caller had already handled.
    pub skipped: u32,
    /// Candidates that failed for a reason other than losing a race (logged).
    pub failed: u32,
    /// Candidates of a shard the replica no longer holds (`FR-010-001`): another replica's now.
    pub fenced: u32,
}

/// One durable collection transition committed by an explicitly fenced scheduler invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionAdvance {
    Entered,
    Admitted {
        index: u32,
    },
    Checkpointed {
        index: u32,
    },
    DrainStarted,
    Drained {
        index: u32,
    },
    DrainFinished,
    Finished,
    /// A compensating scope planned, released or ended its compensation (feature 014, D380).
    Compensated,
    /// The attempted transition retained no value and the run began its durable quota drain.
    QuotaStopped,
    /// A declared hard budget (`ITERATION_LIMIT`, `INSTANCE_LIMIT`) was exceeded before the
    /// collection was entered, and the run was stopped with that durable failure code.
    LimitStopped,
    Waiting,
    AlreadyFinished,
}

/// A collection could not advance without weakening its durable invariants.
#[derive(Debug, thiserror::Error)]
pub enum CollectionAdvanceError {
    #[error("the run does not exist")]
    MissingRun,
    #[error("the scheduler does not hold the run shard")]
    FenceUnavailable,
    #[error("the logical instance is not a collection")]
    NotCollection,
    #[error("durable collection progress does not match the run projection")]
    InvalidProgress,
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error(transparent)]
    Primitive(#[from] PrimitiveStoreError),
    #[error(transparent)]
    Input(#[from] PrimitiveInputError),
    #[error(transparent)]
    Execution(#[from] ExecutionError),
    #[error("collection expression evaluation failed")]
    Expression,
}

/// One durable sequential-loop transition committed by the scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopAdvance {
    Entered,
    Checkpointed {
        index: u32,
    },
    DelayElapsed {
        index: u32,
    },
    DrainStarted,
    Drained {
        index: u32,
    },
    DrainFinished,
    /// A compensating scope planned, released or ended its compensation (feature 014, D380).
    Compensated,
    ExpressionStopped,
    /// Entry retained no value and the run was stopped for its durable-content quota.
    QuotaStopped,
    /// The loop could not be entered within a declared hard budget and the run was stopped with
    /// that durable failure code.
    LimitStopped,
    Waiting,
    AlreadyFinished,
}

/// A sequential loop could not advance without weakening its durable invariants.
#[derive(Debug, thiserror::Error)]
pub enum LoopAdvanceError {
    #[error("the run does not exist")]
    MissingRun,
    #[error("the scheduler does not hold the run shard")]
    FenceUnavailable,
    #[error("the logical instance is not a sequential loop")]
    NotLoop,
    #[error("durable loop progress does not match the run projection")]
    InvalidProgress,
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error(transparent)]
    Progress(#[from] LoopProgressError),
    #[error(transparent)]
    Input(#[from] LoopInputError),
    #[error(transparent)]
    Execution(#[from] ExecutionError),
    #[error("loop expression evaluation failed")]
    Expression,
}

/// A prepared continuation could not be committed without weakening its serialized snapshot.
#[derive(Debug, thiserror::Error)]
pub enum ContinuationAdvanceError {
    #[error("the execution chain does not exist")]
    MissingChain,
    #[error("the scheduler does not hold the current head shard")]
    FenceUnavailable,
    #[error("the requested node is not a pending continuation boundary")]
    NotContinuation,
    #[error("continuation expression evaluation failed")]
    Expression,
    #[error("the continuation expression failed and finalized the chain")]
    ExpressionFinalized,
    #[error("the continuation boundary exhausted its segment limit and finalized the chain")]
    ChainLimitFinalized,
    #[error(transparent)]
    Preparation(#[from] ContinuationPreparationError),
    #[error(transparent)]
    Progress(#[from] ChainProgressError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Engine(#[from] EngineError),
}

impl Report {
    /// Whether anything at all changed.
    #[must_use]
    pub const fn did_work(&self) -> bool {
        self.timers_fired
            + self.collections_advanced
            + self.loops_advanced
            + self.continuations_advanced
            + self.approvals_opened
            + self.approvals_expired
            + self.approval_routes_released
            + self.stopped_runs_settled
            + self.waits_timed_out
            + self.retries_released
            + self.leases_expired
            + self.deadlines_enforced
            + self.hints_renewed
            + self.children_started
            + self.children_finished
            + self.children_cancelled
            + self.quota_stops
            > 0
    }
}

/// What must still be true under the run's lock for a scheduler action to proceed.
#[derive(Clone, Copy)]
enum Condition<'a> {
    Always,
    /// The attempt on `node` is still over: its lease expired (or its own deadline passed) *now*, by
    /// the database clock, not merely when the candidate query ran.
    LeaseLost {
        instance: &'a ExecutionInstanceId,
        cause: LeaseLoss,
    },
}

impl Condition<'_> {
    async fn holds<Tx: RunTx>(&self, tx: &mut Tx) -> Result<bool, StoreError> {
        match self {
            Self::Always => Ok(true),
            Self::LeaseLost { instance, cause } => {
                let now = tx.now();
                Ok(tx
                    .lease_deadlines_instance(instance)
                    .await?
                    .is_some_and(|d| match cause {
                        LeaseLoss::Expired => d.expires_at <= now,
                        LeaseLoss::TimedOut => d.attempt_deadline_at.is_some_and(|at| at <= now),
                    }))
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Outcome {
    Done,
    Skipped,
    Failed,
    /// The replica does not hold the run's shard any more.
    Fenced,
    /// The attempted transition rolled back and the same run was stopped for retained storage.
    QuotaStopped,
}

impl Outcome {
    const fn label(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
            Self::Fenced => "fenced",
            Self::QuotaStopped => "quota_stopped",
        }
    }
}

fn content_quota(error: &EngineError) -> bool {
    matches!(
        error,
        EngineError::Store(error) if content_store_quota(error)
    )
}

fn content_store_quota(error: &StoreError) -> bool {
    matches!(
        error,
        StoreError::QuotaExceeded { what }
            if *what == sagat_domain::quota::QuotaResource::TenantContentBytes.limit_name()
    )
}

fn collection_content_quota(error: &PrimitiveStoreError) -> bool {
    matches!(error, PrimitiveStoreError::Storage(error) if content_store_quota(error))
}

fn loop_content_quota(error: &LoopProgressError) -> bool {
    matches!(error, LoopProgressError::Storage(error) if content_store_quota(error))
}

fn loop_discovery_error(error: LoopProgressError) -> StoreError {
    match error {
        LoopProgressError::Storage(error) => error,
        other => StoreError::Internal {
            detail: format!("loop discovery failed: {other}"),
        },
    }
}

fn recovery_guard(
    shard_held: bool,
    expected_version: u64,
    actual_version: u64,
) -> Result<(), Outcome> {
    if !shard_held {
        return Err(Outcome::Fenced);
    }
    if actual_version != expected_version {
        return Err(Outcome::Skipped);
    }
    Ok(())
}

/// What a tick works on: the shards it may look at, the weights of the tenants, and the snapshot
/// whose leases fence every action. A scheduler with no placement is the only replica: it looks at
/// every shard and fences nothing.
struct View {
    shards: ShardSet,
    weights: Weights,
    snapshot: Option<std::sync::Arc<Snapshot>>,
    owner: String,
}

/// What an action on a run must hold.
enum Hold {
    /// Nothing: the scheduler is the only replica.
    Nothing,
    /// The lease on the run's shard, checked in the action's transaction.
    Lease(Lease),
    /// The replica does not hold the run's shard (any more): the action is another replica's.
    Lost,
}

impl View {
    fn hold_for(&self, run: RunId) -> Hold {
        match &self.snapshot {
            None => Hold::Nothing,
            Some(snapshot) => snapshot
                .lease_of(Shard::of(run.as_uuid()), Instant::now())
                .map_or(Hold::Lost, Hold::Lease),
        }
    }

    /// The fence: inside the transaction that changes the run, the shard is still held at the
    /// generation the replica holds. `true` when the action may go on.
    async fn holds<Tx: RunTx>(&self, tx: &mut Tx, hold: &Hold) -> Result<bool, StoreError> {
        match hold {
            Hold::Nothing => Ok(true),
            Hold::Lease(lease) => tx.holds_shard(&self.owner, *lease).await,
            Hold::Lost => Ok(false),
        }
    }
}

/// The scheduler over a store, an evaluator and a source of jitter.
pub struct Scheduler<S, E, R> {
    store: S,
    engine: Engine<E>,
    entropy: R,
    settings: SchedulerSettings,
    /// Needed to start child runs (it parses the child workflow's definition); without it the
    /// scheduler leaves `child` steps alone.
    definitions: Option<DefinitionCache>,
    /// Which shards this replica works on; without it the scheduler is the only replica.
    placement: Option<Placement>,
    /// Until when each pass rests its search for other tenants (see [`SchedulerSettings::discovery_rest`]).
    rests: std::sync::Mutex<std::collections::BTreeMap<&'static str, Instant>>,
}

impl<S, E, R> std::fmt::Debug for Scheduler<S, E, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scheduler").finish_non_exhaustive()
    }
}

impl<S, E, R> Scheduler<S, E, R>
where
    S: RunStore
        + MaintenanceStore
        + DefinitionStore
        + CollectionStore
        + LoopProgressStore
        + ApprovalSchedulerStore,
    E: ExpressionEvaluator,
    R: Entropy,
{
    pub const fn new(store: S, engine: Engine<E>, entropy: R, settings: SchedulerSettings) -> Self {
        Self {
            store,
            engine,
            entropy,
            settings,
            definitions: None,
            placement: None,
            rests: std::sync::Mutex::new(std::collections::BTreeMap::new()),
        }
    }

    /// Makes the scheduler one of several replicas: it works only on the shards the placement says
    /// it holds, and every action is fenced by the lease it holds on the run's shard.
    #[must_use]
    pub fn with_placement(mut self, placement: Placement) -> Self {
        self.placement = Some(placement);
        self
    }

    fn view(&self) -> View {
        match &self.placement {
            None => View {
                shards: ShardSet::all(),
                weights: Weights::default(),
                snapshot: None,
                owner: String::new(),
            },
            Some(placement) => {
                let snapshot = placement.snapshot();
                View {
                    shards: snapshot.shards(Instant::now()),
                    weights: snapshot.weights().clone(),
                    owner: placement.owner().to_owned(),
                    snapshot: Some(snapshot),
                }
            }
        }
    }

    /// Lets the scheduler start the child runs that `child` steps reserve.
    #[must_use]
    pub fn with_children(mut self, definitions: DefinitionCache) -> Self {
        self.definitions = Some(definitions);
        self
    }

    /// Runs every pass once, over the shards this replica holds (all of them when it is alone).
    pub async fn tick(&self) -> Report
    where
        S: ChainCommandStore + ChainProgressStore,
        S::Tx: ChainRunTx,
    {
        let mut report = Report::default();
        if self.view().shards.is_empty() {
            // A replica that holds nothing (it has not got its share yet, or it cannot reach the
            // database) has nothing to look at; the keeper is what changes that.
            return report;
        }
        // A view for each pass, read when the pass begins: a tick that runs long must go on with the
        // leases the keeper has renewed since, not keep acting on the snapshot it began with until
        // that runs out (every later action would be counted as fenced and the later passes starved).
        self.collections(&self.view(), &mut report).await;
        self.loops(&self.view(), &mut report).await;
        self.continuations(&self.view(), &mut report).await;
        self.approvals_expire(&self.view(), &mut report).await;
        self.approvals_open(&self.view(), &mut report).await;
        self.approval_routes(&self.view(), &mut report).await;
        self.timers(&self.view(), &mut report).await;
        self.wait_deadlines(&self.view(), &mut report).await;
        self.retries(&self.view(), &mut report).await;
        self.leases(&self.view(), &mut report).await;
        self.deadlines(&self.view(), &mut report).await;
        self.stopped_runs(&self.view(), &mut report).await;
        self.hints(&self.view(), &mut report).await;
        self.children_start(&self.view(), &mut report).await;
        self.children_finish(&self.view(), &mut report).await;
        self.children_cancel(&self.view(), &mut report).await;
        report
    }

    /// Advances one collection by one durable transition under the scheduler's current shard
    /// lease. The automatic pass uses the same operation; callers may also request an immediate
    /// transition while worker completions interleave safely.
    ///
    /// # Errors
    /// Returns an error when ownership, projection data, the definition or an expression cannot be
    /// proven consistent.
    pub async fn advance_collection(
        &self,
        tenant: TenantId,
        run: RunId,
        owner: &ExecutionInstanceId,
    ) -> Result<CollectionAdvance, CollectionAdvanceError>
    where
        S: CollectionStore,
    {
        let view = self.view();
        self.advance_collection_in_view(&view, tenant, run, owner)
            .await
    }

    /// Advances one sequential loop by one fenced durable transition.
    ///
    /// `timer_id` is present only when discovery found the current durable delay. It is rechecked
    /// by the transaction before the next body is admitted.
    ///
    /// # Errors
    /// Returns a typed error when the definition, projection, fence or expression snapshot cannot
    /// be proven consistent.
    pub async fn advance_loop(
        &self,
        tenant: TenantId,
        run: RunId,
        owner: &ExecutionInstanceId,
        timer_id: Option<uuid::Uuid>,
    ) -> Result<LoopAdvance, LoopAdvanceError> {
        let view = self.view();
        self.advance_loop_in_view(&view, tenant, run, owner, timer_id)
            .await
    }

    /// Evaluates one pending continuation against a locked snapshot, then commits it only if the
    /// chain, journal and declared-state revisions still match when the write locks are reacquired.
    ///
    /// # Errors
    /// Returns a typed error when the chain is missing, this replica lost its fence, the boundary
    /// is no longer pending, expression evaluation fails, or the atomic write rejects the snapshot.
    pub async fn advance_continuation(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        chain_id: RunId,
        boundary: &NodeId,
    ) -> Result<ChainHandoffWrite, ContinuationAdvanceError>
    where
        S: ChainCommandStore + ChainProgressStore,
        S::Tx: ChainRunTx,
    {
        let view = self.view();
        self.advance_continuation_in_view(&view, tenant, namespace, chain_id, boundary)
            .await
    }

    async fn advance_continuation_in_view(
        &self,
        view: &View,
        tenant: TenantId,
        namespace: NamespaceId,
        chain_id: RunId,
        boundary: &NodeId,
    ) -> Result<ChainHandoffWrite, ContinuationAdvanceError>
    where
        S: ChainCommandStore + ChainProgressStore,
        S::Tx: ChainRunTx,
    {
        let scope = execution::ChainScope::new(tenant, namespace);
        let Some(mut tx) = self.store.begin_chain_head(scope, chain_id).await? else {
            return Err(ContinuationAdvanceError::MissingChain);
        };
        let chain = *tx.locked_chain()?;
        let lease = match view.hold_for(chain.head()) {
            Hold::Lease(lease) => lease,
            Hold::Nothing | Hold::Lost => {
                return Err(ContinuationAdvanceError::FenceUnavailable);
            }
        };
        let info = tx.info().clone();
        let occurred_at = tx.now();
        let data = tx.data().await?.clone();
        let (input_expression, state_expression, expected_run_version) = {
            let (definition, state) = tx.parts();
            let step = state
                .step(boundary)
                .ok_or(ContinuationAdvanceError::NotContinuation)?;
            let Some(NodeKind::ContinueAsNew { input, state: next }) =
                definition.node(boundary).map(|node| node.kind())
            else {
                return Err(ContinuationAdvanceError::NotContinuation);
            };
            if step.status() != execution::StepStatus::Pending {
                return Err(ContinuationAdvanceError::NotContinuation);
            }
            (input.clone(), next.clone(), state.version())
        };
        tx.commit().await?;

        let evaluated = (|| {
            let input = self
                .engine
                .evaluator()
                .child_input(Some(&input_expression), &data)
                .map_err(|_| ContinuationAdvanceError::Expression)?;
            let state = state_expression
                .as_ref()
                .map(|expression| {
                    self.engine
                        .evaluator()
                        .continuation_value(expression, &data)
                        .map_err(|_| ContinuationAdvanceError::Expression)
                })
                .transpose()?;
            Ok::<_, ContinuationAdvanceError>((input, state))
        })();
        let (input, state) = match evaluated {
            Ok(values) => values,
            Err(ContinuationAdvanceError::Expression) => {
                self.finalize_continuation_expression(
                    view,
                    chain,
                    lease,
                    expected_run_version,
                    &info,
                )
                .await?;
                return Err(ContinuationAdvanceError::ExpressionFinalized);
            }
            Err(error) => return Err(error),
        };
        if state.is_some() != data.state_revision.is_some() {
            return Err(ContinuationAdvanceError::NotContinuation);
        }
        let selection =
            ContinuationSelection::new(boundary.clone(), input, state, data.state_revision)?;
        let transfer = selection.transfer();
        let successor = RunId::generate();
        let audit = AuditRecord::engine(
            tenant,
            AuditAction::RunContinued,
            ResourceType::Run,
            chain.head().to_string(),
            "continuation selected",
            info.trace_id.clone(),
            occurred_at,
        );
        let handoff = ChainHandoffCommand::new(
            scope,
            chain_id,
            chain.head(),
            chain.revision(),
            expected_run_version,
            successor,
            transfer,
            SchedulerAuthority::new(view.owner.clone(), lease),
            Subject::engine(),
            info.trace_id.clone(),
            occurred_at,
            audit,
            uuid::Uuid::now_v7(),
            uuid::Uuid::now_v7(),
        );
        match self
            .store
            .continue_evaluated(&selection.into_command(handoff)?)
            .await
        {
            Ok(write) => Ok(write),
            Err(ChainProgressError::Chain(
                sagat_domain::execution::ChainError::SegmentLimitExceeded,
            )) => {
                let Some(mut tx) = self.store.begin_chain_head(scope, chain_id).await? else {
                    return Err(ContinuationAdvanceError::MissingChain);
                };
                let current = *tx.locked_chain()?;
                if current.head() != chain.head() || current.revision() != chain.revision() {
                    return Err(ContinuationAdvanceError::NotContinuation);
                }
                if !view.holds(&mut tx, &Hold::Lease(lease)).await? {
                    return Err(ContinuationAdvanceError::FenceUnavailable);
                }
                let mut effects = Effects::by(Actor::System("continuation"), info.trace_id.clone());
                effects.audit = Some(AuditRecord::engine(
                    tenant,
                    AuditAction::RunFinished,
                    ResourceType::Run,
                    chain.head().to_string(),
                    "chain segment limit exhausted",
                    info.trace_id,
                    tx.now(),
                ));
                self.engine
                    .run(&mut tx, effects, |definition, state, now| {
                        let code = FailureCode::parse("CHAIN_LIMIT")?;
                        Ok((
                            (),
                            execution::stop_run(
                                definition,
                                state,
                                StopReason::Failed,
                                Some(code),
                                now,
                            )?,
                        ))
                    })
                    .await?;
                tx.commit().await?;
                Err(ContinuationAdvanceError::ChainLimitFinalized)
            }
            Err(error) => Err(error.into()),
        }
    }

    async fn finalize_continuation_expression(
        &self,
        view: &View,
        observed: LockedChainHead,
        lease: Lease,
        expected_run_version: u64,
        info: &RunInfo,
    ) -> Result<(), ContinuationAdvanceError>
    where
        S: ChainCommandStore + ChainProgressStore,
        S::Tx: ChainRunTx,
    {
        let Some(mut tx) = self
            .store
            .begin_chain_head(observed.scope(), observed.chain_id())
            .await?
        else {
            return Err(ContinuationAdvanceError::MissingChain);
        };
        let current = *tx.locked_chain()?;
        if current.head() != observed.head()
            || current.revision() != observed.revision()
            || tx.parts().1.version() != expected_run_version
        {
            return Err(ContinuationAdvanceError::NotContinuation);
        }
        if !view.holds(&mut tx, &Hold::Lease(lease)).await? {
            return Err(ContinuationAdvanceError::FenceUnavailable);
        }
        let mut effects = Effects::by(Actor::System("continuation"), info.trace_id.clone());
        effects.audit = Some(AuditRecord::engine(
            observed.scope().tenant_id(),
            AuditAction::RunFinished,
            ResourceType::Run,
            observed.head().to_string(),
            "continuation expression failed",
            info.trace_id.clone(),
            tx.now(),
        ));
        self.engine
            .run(&mut tx, effects, |definition, state, now| {
                Ok((
                    (),
                    execution::stop_run(
                        definition,
                        state,
                        StopReason::Failed,
                        Some(FailureCode::expression_error()),
                        now,
                    )?,
                ))
            })
            .await?;
        tx.commit().await?;
        metrics::counter!("scheduler_continuation_expression_failures_total").increment(1);
        Ok(())
    }

    async fn advance_collection_in_view(
        &self,
        view: &View,
        tenant: TenantId,
        run: RunId,
        owner: &ExecutionInstanceId,
    ) -> Result<CollectionAdvance, CollectionAdvanceError>
    where
        S: CollectionStore,
    {
        let hold = view.hold_for(run);
        let lease = match &hold {
            Hold::Lease(lease) => *lease,
            Hold::Nothing | Hold::Lost => return Err(CollectionAdvanceError::FenceUnavailable),
        };
        let mut tx = self
            .store
            .begin(tenant, run, Lock::Wait)
            .await?
            .ok_or(CollectionAdvanceError::MissingRun)?;
        let info = tx.info().clone();
        let occurred_at = tx.now();
        let mut data = tx.data().await?.clone();
        let (definition, state) = tx.parts();
        let definition = definition.clone();
        let state = state.clone();
        drop(tx);

        let collection_node = definition
            .node_for_instance(owner)
            .ok_or(CollectionAdvanceError::NotCollection)?;
        let NodeKind::ForEach {
            items,
            body,
            result,
            max_items,
            max_concurrency,
        } = collection_node.kind()
        else {
            return Err(CollectionAdvanceError::NotCollection);
        };
        let body = definition
            .body(body)
            .ok_or(CollectionAdvanceError::NotCollection)?;
        let authority = SchedulerAuthority::new(view.owner.clone(), lease);
        let transition = |expected_run_version| {
            CollectionTransition::new(
                tenant,
                info.namespace_id,
                run,
                expected_run_version,
                owner.clone(),
                authority.clone(),
                Subject::engine(),
                info.trace_id.clone(),
                occurred_at,
                None,
            )
        };

        let Some(collection) = state.collection(owner) else {
            let item_values = self
                .engine
                .evaluator()
                .collection_items(items, &data)
                .map_err(|_| CollectionAdvanceError::Expression)?;
            let json_items = item_values
                .iter()
                .map(|item| serde_json::from_slice::<serde_json::Value>(item))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| CollectionAdvanceError::Expression)?;
            let snapshot =
                serde_json::to_vec(&json_items).map_err(|_| CollectionAdvanceError::Expression)?;
            // The captured membership is one value: over the ceiling nothing of it is retained.
            if snapshot.len() > MAX_PAYLOAD_BYTES {
                return self
                    .stop_collection_for_limit(
                        view,
                        &hold,
                        tenant,
                        run,
                        state.version(),
                        FailureCode::run_data_limit(),
                    )
                    .await;
            }
            let limits = CollectionLimits::new(
                *max_items,
                *max_concurrency,
                definition.limits().max_instances(),
                definition.limits().max_depth(),
            )?;
            let entry = match CollectionEntry::from_snapshot(
                transition(state.version()),
                CollectionScopeId::generate(),
                u32::try_from(item_values.len())
                    .map_err(|_| CollectionAdvanceError::InvalidProgress)?,
                snapshot,
                limits,
            ) {
                Ok(entry) => entry.with_iteration_charge(
                    u32::try_from(body.all_nodes().count())
                        .map_err(|_| CollectionAdvanceError::InvalidProgress)?,
                ),
                // More items than the node declared it would iterate over: a hard budget, so the
                // run fails with a durable typed code instead of retrying an entry that cannot
                // succeed.
                Err(PrimitiveInputError::ItemCount) => {
                    return self
                        .stop_collection_for_limit(
                            view,
                            &hold,
                            tenant,
                            run,
                            state.version(),
                            FailureCode::iteration_limit(),
                        )
                        .await;
                }
                Err(error) => return Err(error.into()),
            };
            match self.store.enter_collection(&entry).await {
                Ok(_) => {}
                Err(PrimitiveStoreError::InstanceBudgetExceeded) => {
                    return self
                        .stop_collection_for_limit(
                            view,
                            &hold,
                            tenant,
                            run,
                            state.version(),
                            FailureCode::instance_limit(),
                        )
                        .await;
                }
                Err(error) if collection_content_quota(&error) => {
                    return match self
                        .stop_for_storage_quota(
                            view,
                            &hold,
                            RunRef {
                                tenant_id: tenant,
                                run_id: run,
                            },
                            "collection",
                            Condition::Always,
                            state.version(),
                        )
                        .await?
                    {
                        Outcome::QuotaStopped => Ok(CollectionAdvance::QuotaStopped),
                        Outcome::Skipped => Ok(CollectionAdvance::Waiting),
                        Outcome::Fenced => Err(CollectionAdvanceError::FenceUnavailable),
                        Outcome::Done | Outcome::Failed => Err(CollectionAdvanceError::Engine(
                            EngineError::Store(StoreError::Internal {
                                detail: "unexpected collection quota recovery outcome".to_owned(),
                            }),
                        )),
                    };
                }
                Err(error) => return Err(error.into()),
            }
            return Ok(CollectionAdvance::Entered);
        };

        if matches!(
            collection.phase(),
            execution::CollectionPhase::Succeeded
                | execution::CollectionPhase::Failed
                | execution::CollectionPhase::Cancelled
        ) {
            return Ok(CollectionAdvance::AlreadyFinished);
        }
        if collection.phase() == execution::CollectionPhase::Open {
            let reason = if state.stop_reason() == Some(StopReason::Cancelled) {
                Some(execution::CollectionDrainReason::Cancelled)
            } else {
                collection.active_iterations().find_map(|index| {
                    let path = collection.entry_instance(index)?.path();
                    state.instances().find_map(|(instance, step)| {
                        (instance.path() == path && step.status() == execution::StepStatus::Failed)
                            .then(|| step.failure().cloned())
                            .flatten()
                            .map(|code| execution::CollectionDrainReason::Failed {
                                iteration: index,
                                code,
                            })
                    })
                })
            };
            if let Some(reason) = reason {
                self.store
                    .start_collection_drain(&CollectionDrain::new(
                        transition(state.version()),
                        self.store
                            .collection_progress(tenant, run, owner)
                            .await?
                            .ok_or(CollectionAdvanceError::InvalidProgress)?
                            .scope_id,
                        reason,
                        uuid::Uuid::now_v7(),
                    ))
                    .await?;
                return Ok(CollectionAdvance::DrainStarted);
            }
        } else if collection.phase() == execution::CollectionPhase::Draining {
            let progress = self
                .store
                .collection_progress(tenant, run, owner)
                .await?
                .ok_or(CollectionAdvanceError::InvalidProgress)?;
            if let Some(index) = collection.active_iterations().next() {
                let path = collection
                    .entry_instance(index)
                    .ok_or(CollectionAdvanceError::InvalidProgress)?
                    .path();
                let safe = state
                    .instances()
                    .filter(|(instance, _)| instance.path() == path)
                    .all(|(instance, step)| {
                        let settled_derived = instance.node().is_derived()
                            && matches!(
                                step.status(),
                                execution::StepStatus::Succeeded | execution::StepStatus::Skipped
                            );
                        // Nothing is in flight: a step that has not started, or that waits to
                        // retry (or is ready to) in a run that is stopping and cannot lease it.
                        let safe_unstarted = (step.attempt() == 0
                            && matches!(
                                step.status(),
                                execution::StepStatus::Pending | execution::StepStatus::Ready
                            ))
                            || matches!(
                                step.status(),
                                execution::StepStatus::Ready | execution::StepStatus::RetryWait
                            );
                        step.status().is_terminal() && !instance.node().is_derived()
                            || settled_derived
                            || safe_unstarted
                    });
                if !safe {
                    return Ok(CollectionAdvance::Waiting);
                }
                self.store
                    .drain_collection_iteration(&CollectionIterationDrain::new(
                        transition(state.version()),
                        progress.scope_id,
                        index,
                        uuid::Uuid::now_v7(),
                    ))
                    .await?;
                return Ok(CollectionAdvance::Drained { index });
            }
            if state.instances().any(|(instance, step)| {
                instance != owner && instance.path().is_root() && !step.status().is_terminal()
            }) {
                return Ok(CollectionAdvance::Waiting);
            }
            // A scope that can be compensated undoes what its iterations completed before it ends
            // (D380): the pure planner says what is next, and the commit is the scheduler's own,
            // under its fence, like every other transition of the run.
            if definition.scope_compensates(owner) {
                let finished = state
                    .scope_saga(owner)
                    .is_some_and(|saga| saga.phase() == execution::SagaPhase::Done);
                if !finished {
                    if !matches!(
                        execution::plan_scope_compensation(&definition, &state)?,
                        execution::AddressedPlan::Emit(_)
                    ) {
                        return Ok(CollectionAdvance::Waiting);
                    }
                    return match self
                        .apply_addressed(
                            view,
                            tenant,
                            run,
                            "scope_compensation",
                            self.effects(),
                            |def, state, _| {
                                Ok(((), execution::advance_scope_compensation(def, state)?))
                            },
                        )
                        .await
                    {
                        Outcome::Done => Ok(CollectionAdvance::Compensated),
                        Outcome::Skipped => Ok(CollectionAdvance::Waiting),
                        Outcome::Fenced => Err(CollectionAdvanceError::FenceUnavailable),
                        Outcome::Failed | Outcome::QuotaStopped => {
                            Err(CollectionAdvanceError::Engine(EngineError::Store(
                                StoreError::Internal {
                                    detail: "scope compensation could not be committed".to_owned(),
                                },
                            )))
                        }
                    };
                }
            }
            self.store
                .finish_collection_drain(&CollectionDrainFinish::new(
                    transition(state.version()),
                    progress.scope_id,
                    uuid::Uuid::now_v7(),
                ))
                .await?;
            return Ok(CollectionAdvance::DrainFinished);
        } else {
            return Err(CollectionAdvanceError::InvalidProgress);
        }
        let progress = self
            .store
            .collection_progress(tenant, run, owner)
            .await?
            .ok_or(CollectionAdvanceError::InvalidProgress)?;
        if progress.items.len()
            != usize::try_from(collection.item_count())
                .map_err(|_| CollectionAdvanceError::InvalidProgress)?
        {
            return Err(CollectionAdvanceError::InvalidProgress);
        }

        for index in 0..collection.next_admission() {
            let entry = collection
                .entry_instance(index)
                .ok_or(CollectionAdvanceError::InvalidProgress)?;
            let path = entry.path();
            let pending_return = state.instances().find_map(|(instance, step)| {
                (instance.path() == path
                    && step.status() == execution::StepStatus::Pending
                    && matches!(
                        definition
                            .node_for_instance(instance)
                            .map(|node| node.kind()),
                        Some(NodeKind::Return { .. })
                    ))
                .then_some(instance)
            });
            let Some(return_instance) = pending_return else {
                continue;
            };
            let NodeKind::Return {
                result: return_expression,
            } = definition
                .node_for_instance(return_instance)
                .ok_or(CollectionAdvanceError::InvalidProgress)?
                .kind()
            else {
                return Err(CollectionAdvanceError::InvalidProgress);
            };
            let returned = match self.engine.evaluator().collection_return(
                return_expression,
                &data,
                return_instance,
            ) {
                Ok(returned) => returned,
                // A result that cannot be computed fails this iteration, typed, the way a failed
                // member does; the scope drains instead of being retried forever.
                Err(_) => {
                    return self
                        .fail_collection_iteration(
                            transition(state.version()),
                            progress.scope_id,
                            index,
                            FailureCode::expression_error(),
                        )
                        .await;
                }
            };
            // The iteration's result and, when this is the last one, the aggregate that the
            // finish will store are values too: over 256 KiB, or past what the run may retain,
            // the scope fails here, while the iteration can still fail, and nothing oversize is
            // written. An aggregate that cannot be computed fails the same way, as an expression
            // error, instead of being retried at the finish for ever.
            let refusal = if returned.len() > MAX_PAYLOAD_BYTES {
                Some(FailureCode::run_data_limit())
            } else if progress.results.len() + 1
                == usize::try_from(collection.item_count())
                    .map_err(|_| CollectionAdvanceError::InvalidProgress)?
            {
                self.aggregate_refusal(
                    result,
                    &data,
                    &progress.items,
                    collection
                        .ordered_results()
                        .map(|(position, reference)| (position, reference.return_instance()))
                        .zip(progress.results.iter().map(|(_, bytes)| bytes.as_slice()))
                        .map(|((position, instance), bytes)| (position, instance, bytes)),
                    (index, return_instance, returned.as_slice()),
                )
            } else {
                None
            };
            if let Some(code) = refusal {
                return self
                    .fail_collection_iteration(
                        transition(state.version()),
                        progress.scope_id,
                        index,
                        code,
                    )
                    .await;
            }
            let checkpoint = CollectionCheckpoint::from_result(
                transition(state.version()),
                progress.scope_id,
                index,
                return_instance.clone(),
                returned,
                uuid::Uuid::now_v7(),
            )?;
            match self
                .store
                .checkpoint_collection_iteration(&checkpoint)
                .await
            {
                Ok(_) => {}
                Err(error) if collection_content_quota(&error) => {
                    return self
                        .drain_collection_for_storage_quota(
                            transition(state.version()),
                            progress.scope_id,
                        )
                        .await;
                }
                Err(error) => return Err(error.into()),
            }
            return Ok(CollectionAdvance::Checkpointed { index });
        }

        if collection.next_admission() < collection.item_count()
            && collection.active_count() < usize::from(collection.max_concurrency())
        {
            let index = collection.next_admission();
            let entry = ExecutionInstanceId::in_path(
                owner
                    .path()
                    .child(owner.node().clone(), index)
                    .map_err(|_| CollectionAdvanceError::InvalidProgress)?,
                body.entrypoint().clone(),
            );
            let NodeKind::Activity { activity, .. } = body
                .node(body.entrypoint())
                .ok_or(CollectionAdvanceError::InvalidProgress)?
                .kind()
            else {
                return Err(CollectionAdvanceError::InvalidProgress);
            };
            // The iteration reserves every other node of its body, the compensations and lookups
            // of its activities included, so the budget cannot stop them (D384).
            let reserved_instances = u16::try_from(body.all_nodes().count().saturating_sub(1))
                .map_err(|_| CollectionAdvanceError::InvalidProgress)?;
            let item = progress
                .items
                .get(usize::try_from(index).map_err(|_| CollectionAdvanceError::InvalidProgress)?)
                .ok_or(CollectionAdvanceError::InvalidProgress)?;
            let admission = CollectionAdmission::from_item(
                transition(state.version()),
                progress.scope_id,
                index,
                entry,
                activity.clone(),
                item.clone(),
                reserved_instances,
                uuid::Uuid::now_v7(),
            )?;
            match self.store.admit_collection_activity(&admission).await {
                Ok(_) => {}
                // Entry proved the whole collection fits, so only a later reduction of the run's
                // budget (tenant policy) or other work spending it lands here. Accepted work is
                // kept and no further iteration is admitted until the budget allows it again.
                Err(PrimitiveStoreError::InstanceBudgetExceeded) => {
                    metrics::counter!(
                        "scheduler_admission_paused_total",
                        "kind" => "collection",
                        "reason" => "instance_budget",
                    )
                    .increment(1);
                    return Ok(CollectionAdvance::Waiting);
                }
                Err(error) if collection_content_quota(&error) => {
                    return self
                        .drain_collection_for_storage_quota(
                            transition(state.version()),
                            progress.scope_id,
                        )
                        .await;
                }
                Err(error) => return Err(error.into()),
            }
            return Ok(CollectionAdvance::Admitted { index });
        }

        if collection.next_admission() == collection.item_count() && collection.active_count() == 0
        {
            let mut mapped = Vec::with_capacity(progress.results.len());
            for (index, return_ref) in collection.ordered_results() {
                let (stored_index, body_result) = progress
                    .results
                    .get(mapped.len())
                    .ok_or(CollectionAdvanceError::InvalidProgress)?;
                if *stored_index != index {
                    return Err(CollectionAdvanceError::InvalidProgress);
                }
                // The iteration ended, so the run data no longer carries its input; the snapshot does.
                // The result expression reads `item` and `index` as the body did.
                let item = progress
                    .items
                    .get(
                        usize::try_from(index)
                            .map_err(|_| CollectionAdvanceError::InvalidProgress)?,
                    )
                    .ok_or(CollectionAdvanceError::InvalidProgress)?;
                data.scope_inputs.insert(
                    return_ref.return_instance().path().clone(),
                    ScopeInput::Collection {
                        index,
                        item: item.clone(),
                    },
                );
                let value = self
                    .engine
                    .evaluator()
                    .collection_member_result(
                        result,
                        &data,
                        return_ref.return_instance(),
                        body_result,
                    )
                    .map_err(|_| CollectionAdvanceError::Expression)?;
                mapped.push(
                    serde_json::from_slice::<serde_json::Value>(&value)
                        .map_err(|_| CollectionAdvanceError::Expression)?,
                );
            }
            if mapped.len()
                != usize::try_from(collection.item_count())
                    .map_err(|_| CollectionAdvanceError::InvalidProgress)?
            {
                return Err(CollectionAdvanceError::InvalidProgress);
            }
            let aggregate =
                serde_json::to_vec(&mapped).map_err(|_| CollectionAdvanceError::Expression)?;
            let finish = CollectionFinish::from_evaluated_result(
                transition(state.version()),
                progress.scope_id,
                aggregate,
                &progress.results,
                uuid::Uuid::now_v7(),
            );
            match self.store.finish_collection(&finish).await {
                Ok(_) => {}
                Err(error) if collection_content_quota(&error) => {
                    return self
                        .drain_collection_for_storage_quota(
                            transition(state.version()),
                            progress.scope_id,
                        )
                        .await;
                }
                Err(error) => return Err(error.into()),
            }
            return Ok(CollectionAdvance::Finished);
        }

        Ok(CollectionAdvance::Waiting)
    }

    /// Fails one admitted iteration whose result cannot be produced or retained: the scope drains
    /// with that iteration's typed failure, as it does for a failed member.
    async fn fail_collection_iteration(
        &self,
        transition: CollectionTransition,
        scope_id: CollectionScopeId,
        index: u32,
        code: FailureCode,
    ) -> Result<CollectionAdvance, CollectionAdvanceError> {
        self.store
            .start_collection_drain(&CollectionDrain::new(
                transition,
                scope_id,
                execution::CollectionDrainReason::Failed {
                    iteration: index,
                    code,
                },
                uuid::Uuid::now_v7(),
            ))
            .await?;
        Ok(CollectionAdvance::DrainStarted)
    }

    /// Why the aggregate the collection would store, given the results so far and the one being
    /// checkpointed, must not be stored: it cannot be computed (an expression error), it is over
    /// the 256 KiB value ceiling, or it would push the run's retained outputs past 4 MiB (both
    /// `RUN_DATA_LIMIT`). `None` when it fits.
    fn aggregate_refusal<'a>(
        &self,
        result: &sagat_domain::workflow::Expression,
        data: &sagat_application::execution::RunData,
        items: &[Vec<u8>],
        stored: impl Iterator<Item = (u32, &'a ExecutionInstanceId, &'a [u8])>,
        current: (u32, &'a ExecutionInstanceId, &'a [u8]),
    ) -> Option<FailureCode> {
        let mut members: Vec<_> = stored.collect();
        members.push(current);
        members.sort_by_key(|(position, _, _)| *position);
        // The iterations that already ended no longer carry their input in the run data; the
        // snapshot does, and the result expression reads `item` and `index` as the body did.
        let mut data = data.clone();
        for (position, instance, _) in &members {
            // A position the snapshot does not hold is an inconsistent progress record: refuse it
            // as an expression error instead of letting the aggregate pass unchecked.
            let Some(item) = usize::try_from(*position)
                .ok()
                .and_then(|position| items.get(position))
            else {
                return Some(FailureCode::expression_error());
            };
            data.scope_inputs.insert(
                instance.path().clone(),
                ScopeInput::Collection {
                    index: *position,
                    item: item.clone(),
                },
            );
        }
        let mut mapped = Vec::with_capacity(members.len());
        for (_, instance, body_result) in members {
            let value = self
                .engine
                .evaluator()
                .collection_member_result(result, &data, instance, body_result)
                .ok()
                .and_then(|value| serde_json::from_slice::<serde_json::Value>(&value).ok());
            let Some(value) = value else {
                return Some(FailureCode::expression_error());
            };
            mapped.push(value);
        }
        let Ok(aggregate) = serde_json::to_vec(&mapped) else {
            return Some(FailureCode::expression_error());
        };
        let retained: usize = data.outputs.values().map(Vec::len).sum();
        (aggregate.len() > MAX_PAYLOAD_BYTES
            || retained.saturating_add(aggregate.len()) > MAX_RUN_OUTPUT_BYTES)
            .then(FailureCode::run_data_limit)
    }

    /// Stops the run before its collection starts because a declared hard budget does not allow
    /// it: the run fails with `code`, journaled and typed, and nothing was admitted.
    async fn stop_collection_for_limit(
        &self,
        view: &View,
        hold: &Hold,
        tenant: TenantId,
        run: RunId,
        version: u64,
        code: FailureCode,
    ) -> Result<CollectionAdvance, CollectionAdvanceError> {
        match self
            .stop_for_limit(
                view,
                hold,
                RunRef {
                    tenant_id: tenant,
                    run_id: run,
                },
                WorkflowPrimitiveKind::Collection,
                code,
                version,
            )
            .await?
        {
            Outcome::Done => Ok(CollectionAdvance::LimitStopped),
            Outcome::Skipped => Ok(CollectionAdvance::Waiting),
            Outcome::Fenced => Err(CollectionAdvanceError::FenceUnavailable),
            Outcome::Failed | Outcome::QuotaStopped => Err(CollectionAdvanceError::Engine(
                EngineError::Store(StoreError::Internal {
                    detail: "unexpected collection limit recovery outcome".to_owned(),
                }),
            )),
        }
    }

    async fn drain_collection_for_storage_quota(
        &self,
        transition: CollectionTransition,
        scope_id: CollectionScopeId,
    ) -> Result<CollectionAdvance, CollectionAdvanceError> {
        let audit = AuditRecord::engine(
            transition.tenant_id(),
            AuditAction::RunStorageQuotaExceeded,
            ResourceType::Run,
            transition.run_id().to_string(),
            "collection storage quota exceeded",
            transition.trace_id().map(str::to_owned),
            transition.occurred_at(),
        );
        self.store
            .start_collection_drain(&CollectionDrain::new(
                transition.with_audit(audit),
                scope_id,
                execution::CollectionDrainReason::StorageQuotaExceeded,
                uuid::Uuid::now_v7(),
            ))
            .await?;
        metrics::counter!(
            "scheduler_quota_stops_total",
            "limit" => sagat_domain::quota::QuotaResource::TenantContentBytes.limit_name(),
            "kind" => "collection",
        )
        .increment(1);
        Ok(CollectionAdvance::QuotaStopped)
    }

    async fn advance_loop_in_view(
        &self,
        view: &View,
        tenant: TenantId,
        run: RunId,
        owner: &ExecutionInstanceId,
        timer_id: Option<uuid::Uuid>,
    ) -> Result<LoopAdvance, LoopAdvanceError> {
        let hold = view.hold_for(run);
        let lease = match &hold {
            Hold::Lease(lease) => *lease,
            Hold::Nothing | Hold::Lost => return Err(LoopAdvanceError::FenceUnavailable),
        };
        let mut tx = self
            .store
            .begin(tenant, run, Lock::Wait)
            .await?
            .ok_or(LoopAdvanceError::MissingRun)?;
        let info = tx.info().clone();
        let occurred_at = tx.now();
        let data = tx.data().await?.clone();
        let (definition, state) = tx.parts();
        let definition = definition.clone();
        let state = state.clone();
        drop(tx);

        let loop_node = definition
            .node_for_instance(owner)
            .ok_or(LoopAdvanceError::NotLoop)?;
        let (kind, condition, body_name, initial, carry, max_iterations, delay) =
            match loop_node.kind() {
                NodeKind::While {
                    condition,
                    body,
                    initial,
                    carry,
                    max_iterations,
                    delay,
                } => (
                    LoopKind::While,
                    condition,
                    body,
                    initial,
                    carry,
                    *max_iterations,
                    *delay,
                ),
                NodeKind::RepeatUntil {
                    condition,
                    body,
                    initial,
                    carry,
                    max_iterations,
                    delay,
                } => (
                    LoopKind::RepeatUntil,
                    condition,
                    body,
                    initial,
                    carry,
                    *max_iterations,
                    *delay,
                ),
                _ => return Err(LoopAdvanceError::NotLoop),
            };
        let timeout = loop_node.timeout().ok_or(LoopAdvanceError::NotLoop)?;
        let body = definition
            .body(body_name)
            .ok_or(LoopAdvanceError::NotLoop)?;
        let next_activity = |index| -> Result<LoopNextActivity, LoopAdvanceError> {
            let entry = body
                .node(body.entrypoint())
                .ok_or(LoopAdvanceError::InvalidProgress)?;
            let NodeKind::Activity { activity, .. } = entry.kind() else {
                return Err(LoopAdvanceError::InvalidProgress);
            };
            let path = owner
                .path()
                .child(owner.node().clone(), index)
                .map_err(|_| LoopAdvanceError::InvalidProgress)?;
            let reserved_instances = body
                .all_nodes()
                .count()
                .checked_sub(1)
                .and_then(|count| u16::try_from(count).ok())
                .ok_or(LoopAdvanceError::InvalidProgress)?;
            Ok(LoopNextActivity::new(
                ExecutionInstanceId::in_path(path, body.entrypoint().clone()),
                activity.clone(),
                reserved_instances,
            )?)
        };
        let authority = SchedulerAuthority::new(view.owner.clone(), lease);
        let transition = |expected_run_version| {
            LoopTransition::new(
                tenant,
                info.namespace_id,
                run,
                expected_run_version,
                owner.clone(),
                authority.clone(),
                Subject::engine(),
                info.trace_id.clone(),
                occurred_at,
                AuditRecord::engine(
                    tenant,
                    AuditAction::RunUpdated,
                    ResourceType::Run,
                    run.to_string(),
                    "sequential loop advanced",
                    info.trace_id.clone(),
                    occurred_at,
                ),
            )
        };

        let Some(loop_state) = state.loop_state(owner) else {
            let evaluated = match self.engine.evaluator().loop_entry(
                initial,
                (kind == LoopKind::While).then_some(condition),
                &data,
            ) {
                Ok(evaluated) => evaluated,
                Err(_) => {
                    return match self
                        .stop_loop_for_expression(
                            view,
                            &hold,
                            RunRef {
                                tenant_id: tenant,
                                run_id: run,
                            },
                            state.version(),
                        )
                        .await?
                    {
                        Outcome::Done => Ok(LoopAdvance::ExpressionStopped),
                        Outcome::Skipped => Ok(LoopAdvance::Waiting),
                        Outcome::Fenced => Err(LoopAdvanceError::FenceUnavailable),
                        Outcome::Failed | Outcome::QuotaStopped => {
                            Err(LoopAdvanceError::Expression)
                        }
                    };
                }
            };
            // The initial carry is a value: over the ceiling nothing of it is retained.
            if evaluated.carry.len() > MAX_PAYLOAD_BYTES {
                return match self
                    .stop_for_limit(
                        view,
                        &hold,
                        RunRef {
                            tenant_id: tenant,
                            run_id: run,
                        },
                        WorkflowPrimitiveKind::Loop,
                        FailureCode::run_data_limit(),
                        state.version(),
                    )
                    .await?
                {
                    Outcome::Done => Ok(LoopAdvance::LimitStopped),
                    Outcome::Skipped => Ok(LoopAdvance::Waiting),
                    Outcome::Fenced => Err(LoopAdvanceError::FenceUnavailable),
                    Outcome::Failed | Outcome::QuotaStopped => Err(LoopAdvanceError::Engine(
                        EngineError::Store(StoreError::Internal {
                            detail: "unexpected loop limit recovery outcome".to_owned(),
                        }),
                    )),
                };
            }
            let needs_body = kind == LoopKind::RepeatUntil || evaluated.condition == Some(true);
            let first_activity = needs_body.then(|| next_activity(0)).transpose()?;
            let entry = LoopEntryCommand::from_carry(
                transition(state.version()),
                kind,
                max_iterations,
                occurred_at + timeout,
                evaluated.carry,
                evaluated.condition,
                first_activity,
                uuid::Uuid::now_v7(),
            )?;
            match self.store.enter_loop(&entry).await {
                Ok(_) => {}
                // The owner, the first body and what follows the loop do not fit the run's
                // lifetime instance budget: nothing was entered, and the run fails typed.
                Err(LoopProgressError::InstanceBudgetExceeded) => {
                    return match self
                        .stop_for_limit(
                            view,
                            &hold,
                            RunRef {
                                tenant_id: tenant,
                                run_id: run,
                            },
                            WorkflowPrimitiveKind::Loop,
                            FailureCode::instance_limit(),
                            state.version(),
                        )
                        .await?
                    {
                        Outcome::Done => Ok(LoopAdvance::LimitStopped),
                        Outcome::Skipped => Ok(LoopAdvance::Waiting),
                        Outcome::Fenced => Err(LoopAdvanceError::FenceUnavailable),
                        Outcome::Failed | Outcome::QuotaStopped => Err(LoopAdvanceError::Engine(
                            EngineError::Store(StoreError::Internal {
                                detail: "unexpected loop limit recovery outcome".to_owned(),
                            }),
                        )),
                    };
                }
                Err(error) if loop_content_quota(&error) => {
                    return match self
                        .stop_for_storage_quota(
                            view,
                            &hold,
                            RunRef {
                                tenant_id: tenant,
                                run_id: run,
                            },
                            "loop",
                            Condition::Always,
                            state.version(),
                        )
                        .await?
                    {
                        Outcome::QuotaStopped => Ok(LoopAdvance::QuotaStopped),
                        Outcome::Skipped => Ok(LoopAdvance::Waiting),
                        Outcome::Fenced => Err(LoopAdvanceError::FenceUnavailable),
                        Outcome::Done | Outcome::Failed => Err(LoopAdvanceError::Engine(
                            EngineError::Store(StoreError::Internal {
                                detail: "unexpected loop quota recovery outcome".to_owned(),
                            }),
                        )),
                    };
                }
                Err(error) => return Err(error.into()),
            }
            return Ok(LoopAdvance::Entered);
        };

        if matches!(
            loop_state.phase(),
            LoopPhase::Succeeded | LoopPhase::Failed | LoopPhase::Cancelled
        ) {
            return Ok(LoopAdvance::AlreadyFinished);
        }
        if loop_state.phase() == LoopPhase::Draining {
            if let Some(index) = loop_state.active_iteration() {
                let path = owner
                    .path()
                    .child(owner.node().clone(), index)
                    .map_err(|_| LoopAdvanceError::InvalidProgress)?;
                let safe = state
                    .instances()
                    .filter(|(instance, _)| instance.path() == &path)
                    .all(|(instance, step)| {
                        let settled_derived = instance.node().is_derived()
                            && matches!(
                                step.status(),
                                execution::StepStatus::Succeeded | execution::StepStatus::Skipped
                            );
                        // Nothing is in flight: a step that has not started, or one that waits
                        // to retry.
                        let safe_unstarted = (step.attempt() == 0
                            && matches!(
                                step.status(),
                                execution::StepStatus::Pending | execution::StepStatus::Ready
                            ))
                            || step.status() == execution::StepStatus::RetryWait;
                        step.status().is_terminal() && !instance.node().is_derived()
                            || settled_derived
                            || safe_unstarted
                    });
                if !safe {
                    return Ok(LoopAdvance::Waiting);
                }
                self.store
                    .drain_iteration(&LoopIterationDrainCommand::new(
                        transition(state.version()),
                        index,
                        uuid::Uuid::now_v7(),
                    )?)
                    .await?;
                return Ok(LoopAdvance::Drained { index });
            }
            if state.instances().any(|(instance, step)| {
                instance != owner && instance.path().is_root() && !step.status().is_terminal()
            }) {
                return Ok(LoopAdvance::Waiting);
            }
            // A loop that can be compensated undoes what its iterations completed before it ends,
            // last iteration first (D380, D383).
            if definition.scope_compensates(owner) {
                let finished = state
                    .scope_saga(owner)
                    .is_some_and(|saga| saga.phase() == execution::SagaPhase::Done);
                if !finished {
                    if !matches!(
                        execution::plan_scope_compensation(&definition, &state)?,
                        execution::AddressedPlan::Emit(_)
                    ) {
                        return Ok(LoopAdvance::Waiting);
                    }
                    return match self
                        .apply_addressed(
                            view,
                            tenant,
                            run,
                            "scope_compensation",
                            self.effects(),
                            |def, state, _| {
                                Ok(((), execution::advance_scope_compensation(def, state)?))
                            },
                        )
                        .await
                    {
                        Outcome::Done => Ok(LoopAdvance::Compensated),
                        Outcome::Skipped => Ok(LoopAdvance::Waiting),
                        Outcome::Fenced => Err(LoopAdvanceError::FenceUnavailable),
                        Outcome::Failed | Outcome::QuotaStopped => Err(LoopAdvanceError::Engine(
                            EngineError::Store(StoreError::Internal {
                                detail: "scope compensation could not be committed".to_owned(),
                            }),
                        )),
                    };
                }
            }
            self.store
                .finish_drain(&LoopDrainFinishCommand::new(
                    transition(state.version()),
                    uuid::Uuid::now_v7(),
                ))
                .await?;
            return Ok(LoopAdvance::DrainFinished);
        }
        if state.stop_reason() == Some(StopReason::Cancelled) {
            self.store
                .start_drain(&LoopDrainCommand::new(
                    transition(state.version()),
                    execution::LoopDrainReason::Cancelled,
                    uuid::Uuid::now_v7(),
                )?)
                .await?;
            return Ok(LoopAdvance::DrainStarted);
        }
        if occurred_at >= loop_state.deadline() {
            self.store
                .start_drain(&LoopDrainCommand::new(
                    transition(state.version()),
                    execution::LoopDrainReason::TimedOut,
                    uuid::Uuid::now_v7(),
                )?)
                .await?;
            return Ok(LoopAdvance::DrainStarted);
        }

        if loop_state.phase() == LoopPhase::Delaying {
            let due_at = loop_state
                .delay_until()
                .ok_or(LoopAdvanceError::InvalidProgress)?;
            if due_at > occurred_at {
                return Ok(LoopAdvance::Waiting);
            }
            let timer_id = timer_id.ok_or(LoopAdvanceError::InvalidProgress)?;
            let index = loop_state.next_iteration();
            match self
                .store
                .fire_delay(&LoopDelayElapsedCommand::new(
                    transition(state.version()),
                    timer_id,
                    index,
                    next_activity(index)?,
                    uuid::Uuid::now_v7(),
                )?)
                .await
            {
                Ok(_) => {}
                // The budget was reduced while the loop waited: admission pauses, accepted work
                // is kept, and the loop's own deadline still bounds the wait.
                Err(LoopProgressError::InstanceBudgetExceeded) => {
                    metrics::counter!(
                        "scheduler_admission_paused_total",
                        "kind" => "loop",
                        "reason" => "instance_budget",
                    )
                    .increment(1);
                    return Ok(LoopAdvance::Waiting);
                }
                Err(error) => return Err(error.into()),
            }
            return Ok(LoopAdvance::DelayElapsed { index });
        }

        if loop_state.phase() != LoopPhase::Running {
            return Err(LoopAdvanceError::InvalidProgress);
        }
        let index = loop_state
            .active_iteration()
            .ok_or(LoopAdvanceError::InvalidProgress)?;
        let member_path = owner
            .path()
            .child(owner.node().clone(), index)
            .map_err(|_| LoopAdvanceError::InvalidProgress)?;
        if let Some(code) = state.instances().find_map(|(instance, step)| {
            (instance.path() == &member_path && step.status() == execution::StepStatus::Failed)
                .then(|| step.failure().cloned())
                .flatten()
        }) {
            self.store
                .start_drain(&LoopDrainCommand::new(
                    transition(state.version()),
                    execution::LoopDrainReason::Failed {
                        iteration: index,
                        code,
                    },
                    uuid::Uuid::now_v7(),
                )?)
                .await?;
            return Ok(LoopAdvance::DrainStarted);
        }
        let return_instance = state.instances().find_map(|(instance, step)| {
            (instance.path() == &member_path
                && step.status() == execution::StepStatus::Pending
                && matches!(
                    definition
                        .node_for_instance(instance)
                        .map(sagat_domain::workflow::Node::kind),
                    Some(NodeKind::Return { .. })
                ))
            .then_some(instance.clone())
        });
        let Some(return_instance) = return_instance else {
            return Ok(LoopAdvance::Waiting);
        };
        let NodeKind::Return { result } = definition
            .node_for_instance(&return_instance)
            .ok_or(LoopAdvanceError::InvalidProgress)?
            .kind()
        else {
            return Err(LoopAdvanceError::InvalidProgress);
        };
        let evaluated = match self.engine.evaluator().loop_checkpoint(
            result,
            carry,
            condition,
            &data,
            &return_instance,
        ) {
            Ok(evaluated) => evaluated,
            Err(_) => {
                self.store
                    .start_drain(&LoopDrainCommand::new(
                        transition(state.version()),
                        execution::LoopDrainReason::Failed {
                            iteration: index,
                            code: FailureCode::expression_error(),
                        },
                        uuid::Uuid::now_v7(),
                    )?)
                    .await?;
                return Ok(LoopAdvance::DrainStarted);
            }
        };
        let next_index = index
            .checked_add(1)
            .ok_or(LoopAdvanceError::InvalidProgress)?;
        let continues = match kind {
            LoopKind::While => evaluated.condition,
            LoopKind::RepeatUntil => !evaluated.condition,
        };
        // The result and the carry are values: over 256 KiB nothing is retained and the loop fails
        // at this iteration's result boundary, as it does for an expression that cannot be
        // computed. The carry that ends the loop becomes its output, which the run must be able to
        // retain beside every other output.
        let retained: usize = data.outputs.values().map(Vec::len).sum();
        if evaluated.result.len() > MAX_PAYLOAD_BYTES
            || evaluated.carry.len() > MAX_PAYLOAD_BYTES
            || (!continues && retained.saturating_add(evaluated.carry.len()) > MAX_RUN_OUTPUT_BYTES)
        {
            self.store
                .start_drain(&LoopDrainCommand::new(
                    transition(state.version()),
                    execution::LoopDrainReason::Failed {
                        iteration: index,
                        code: FailureCode::run_data_limit(),
                    },
                    uuid::Uuid::now_v7(),
                )?)
                .await?;
            return Ok(LoopAdvance::DrainStarted);
        }
        // What the next body would charge: one instance per node, charged when it is admitted.
        let budget = definition.limits().max_instances();
        let next_charge = body.nodes().count();
        let affordable = state
            .charged_instances()
            .checked_add(next_charge)
            .and_then(|total| u32::try_from(total).ok())
            .is_some_and(|total| total <= budget);
        let successor_affordable = state
            .charged_instances()
            .checked_add(1)
            .and_then(|total| u32::try_from(total).ok())
            .is_some_and(|total| total <= budget);
        let (continuation, next, delay_timer) = if !continues {
            if successor_affordable {
                (LoopContinuation::Complete, None, None)
            } else {
                (LoopContinuation::InstanceLimit, None, None)
            }
        } else if next_index >= max_iterations {
            (LoopContinuation::IterationLimit, None, None)
        } else if !affordable {
            // Needing another body the author's instance limit cannot pay for is a durable,
            // typed stop, decided before any delay is scheduled for work that can never run.
            (LoopContinuation::InstanceLimit, None, None)
        } else if let Some(delay) = delay {
            let due_at = occurred_at + delay;
            if due_at > loop_state.deadline() {
                self.store
                    .start_drain(&LoopDrainCommand::new(
                        transition(state.version()),
                        execution::LoopDrainReason::TimedOut,
                        uuid::Uuid::now_v7(),
                    )?)
                    .await?;
                return Ok(LoopAdvance::DrainStarted);
            }
            (
                LoopContinuation::DelayUntil(due_at),
                None,
                Some(uuid::Uuid::now_v7()),
            )
        } else {
            (
                LoopContinuation::Continue,
                Some(next_activity(next_index)?),
                None,
            )
        };
        let checkpoint = LoopCheckpointCommand::from_evaluation(
            transition(state.version()),
            index,
            return_instance.clone(),
            evaluated.clone(),
            occurred_at,
            continuation,
            next,
            delay_timer,
            uuid::Uuid::now_v7(),
        )?;
        let mut committed = self.store.checkpoint(&checkpoint).await;
        // The store knows the run's real budget, which tenant policy may have lowered below the
        // author's: when it cannot admit what the checkpoint needs next, the same checkpoint is
        // committed as the typed stop instead, which it re-proves against that budget.
        if matches!(committed, Err(LoopProgressError::InstanceBudgetExceeded))
            && matches!(
                continuation,
                LoopContinuation::Continue | LoopContinuation::Complete
            )
        {
            let refused = LoopCheckpointCommand::from_evaluation(
                transition(state.version()),
                index,
                return_instance,
                evaluated,
                occurred_at,
                LoopContinuation::InstanceLimit,
                None,
                None,
                uuid::Uuid::now_v7(),
            )?;
            committed = self.store.checkpoint(&refused).await;
        }
        match committed {
            Ok(_) => {}
            Err(error) if loop_content_quota(&error) => {
                return match self
                    .stop_for_storage_quota(
                        view,
                        &hold,
                        RunRef {
                            tenant_id: tenant,
                            run_id: run,
                        },
                        "loop",
                        Condition::Always,
                        state.version(),
                    )
                    .await?
                {
                    Outcome::QuotaStopped => Ok(LoopAdvance::QuotaStopped),
                    Outcome::Skipped => Ok(LoopAdvance::Waiting),
                    Outcome::Fenced => Err(LoopAdvanceError::FenceUnavailable),
                    Outcome::Done | Outcome::Failed => Err(LoopAdvanceError::Engine(
                        EngineError::Store(StoreError::Internal {
                            detail: "unexpected loop quota recovery outcome".to_owned(),
                        }),
                    )),
                };
            }
            Err(error) => return Err(error.into()),
        }
        Ok(LoopAdvance::Checkpointed { index })
    }

    /// Purges expired idempotency records and worker sessions. Run it rarely.
    pub async fn housekeeping(&self) {
        match self
            .store
            .purge_idempotency(self.settings.idempotency_retention)
            .await
        {
            Ok(n) if n > 0 => tracing::info!(removed = n, "purged idempotency records"),
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "could not purge idempotency records"),
        }
        match self.store.purge_sessions().await {
            Ok(n) if n > 0 => tracing::info!(removed = n, "purged expired worker sessions"),
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "could not purge worker sessions"),
        }
        // The records scripts keep for a while (executions, test runs) end with their retention.
        let mut scripts = 0u64;
        for _ in 0..PURGE_ROUNDS {
            match self.store.purge_script_records(PURGE_BATCH).await {
                Ok(n) => {
                    scripts += n;
                    if n < u64::from(PURGE_BATCH) {
                        break;
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "could not purge script records");
                    break;
                }
            }
        }
        if scripts > 0 {
            tracing::info!(
                removed = scripts,
                "purged script records past their retention"
            );
        }
        // One bounded statement at a time, so a backlog never holds a long one, repeated until the
        // backlog is gone (or this call has done its share): the purge must outrun what is written.
        let mut removed = 0u64;
        for _ in 0..PURGE_ROUNDS {
            match self
                .store
                .purge_outbox(self.settings.outbox_retention, PURGE_BATCH)
                .await
            {
                Ok(n) => {
                    removed += n;
                    if n < u64::from(PURGE_BATCH) {
                        break;
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "could not purge the outbox");
                    break;
                }
            }
        }
        if removed > 0 {
            tracing::info!(removed, "purged delivered outbox messages");
        }
    }

    /// Applies `command` to a run under its row lock and commits, provided `condition` still holds
    /// there. The candidate queries read without a lock, so what made a run a candidate may have
    /// changed since (a heartbeat renewed the lease, a retry was already released): the check
    /// is repeated under the lock, and a candidate that no longer qualifies is skipped.
    async fn apply_if<F>(
        &self,
        view: &View,
        RunRef {
            tenant_id: tenant,
            run_id: run,
        }: RunRef,
        kind: &'static str,
        condition: Condition<'_>,
        effects: Effects,
        command: F,
    ) -> Outcome
    where
        F: FnOnce(
            &sagat_domain::workflow::WorkflowDefinition,
            &mut sagat_domain::execution::RunState,
            OffsetDateTime,
        ) -> Result<((), Progress), ExecutionError>,
    {
        let hold = view.hold_for(run);
        let outcome = async {
            if matches!(hold, Hold::Lost) {
                // The shard changed hands after the candidate query: do not even open a transaction.
                return Ok(Outcome::Fenced);
            }
            let Some(mut tx) = self
                .store
                .begin(tenant, run, Lock::Wait)
                .await
                .map_err(EngineError::Store)?
            else {
                return Ok(Outcome::Skipped);
            };
            // The work belongs to the trace of the run it acts on, under the run's span (D162):
            // what the scheduler did to a run is read next to what the run did.
            let trace = tx.info().trace_id.clone();
            let span = tracing::info_span!(
                "scheduler.action",
                kind,
                run_id = %run,
                tenant_id = %tenant,
                trace_id = trace.as_deref(),
                parent_span_id = %sagat_telemetry::run_span_id(run.as_uuid()),
            );
            async {
                // The fence comes first: whatever the run's state says, it is not this replica's to
                // change unless it still holds the shard at the generation it believes it does.
                if !view
                    .holds(&mut tx, &hold)
                    .await
                    .map_err(EngineError::Store)?
                {
                    return Ok(Outcome::Fenced);
                }
                if !condition.holds(&mut tx).await.map_err(EngineError::Store)? {
                    return Ok(Outcome::Skipped);
                }
                let version = tx.parts().1.version();
                match self.engine.run(&mut tx, effects, command).await {
                    Ok(()) => match tx.commit().await.map_err(EngineError::Store) {
                        Ok(()) => Ok(Outcome::Done),
                        Err(error) if content_quota(&error) => {
                            self.stop_for_storage_quota(
                                view,
                                &hold,
                                RunRef {
                                    tenant_id: tenant,
                                    run_id: run,
                                },
                                kind,
                                condition,
                                version,
                            )
                            .await
                        }
                        Err(error) => Err(error),
                    },
                    // The domain refused: the state moved since the candidate query (a worker
                    // answered, a signal arrived, the run was cancelled). Dropping `tx` rolls back.
                    Err(EngineError::Execution(error))
                        if !matches!(error, ExecutionError::Invariant(_)) =>
                    {
                        Ok(Outcome::Skipped)
                    }
                    Err(error) if content_quota(&error) => {
                        drop(tx);
                        self.stop_for_storage_quota(
                            view,
                            &hold,
                            RunRef {
                                tenant_id: tenant,
                                run_id: run,
                            },
                            kind,
                            condition,
                            version,
                        )
                        .await
                    }
                    Err(other) => Err(other),
                }
            }
            .instrument(span)
            .await
        }
        .await;
        let outcome = outcome.unwrap_or_else(|error: EngineError| {
            tracing::error!(%error, %run, kind, "a scheduler action failed");
            Outcome::Failed
        });
        metrics::counter!("scheduler_actions_total", "kind" => kind, "outcome" => outcome.label())
            .increment(1);
        if outcome == Outcome::Fenced {
            metrics::counter!("scheduler_fenced_total", "kind" => kind).increment(1);
        }
        outcome
    }

    async fn apply_addressed_if<F>(
        &self,
        view: &View,
        RunRef {
            tenant_id: tenant,
            run_id: run,
        }: RunRef,
        kind: &'static str,
        condition: Condition<'_>,
        effects: Effects,
        command: F,
    ) -> Outcome
    where
        F: FnOnce(
            &sagat_domain::workflow::WorkflowDefinition,
            &mut sagat_domain::execution::RunState,
            OffsetDateTime,
        ) -> Result<((), AddressedProgress), ExecutionError>,
    {
        let hold = view.hold_for(run);
        let outcome = async {
            if matches!(hold, Hold::Lost) {
                return Ok(Outcome::Fenced);
            }
            let Some(mut tx) = self
                .store
                .begin(tenant, run, Lock::Wait)
                .await
                .map_err(EngineError::Store)?
            else {
                return Ok(Outcome::Skipped);
            };
            let trace = tx.info().trace_id.clone();
            let span = tracing::info_span!(
                "scheduler.action",
                kind,
                run_id = %run,
                tenant_id = %tenant,
                trace_id = trace.as_deref(),
                parent_span_id = %sagat_telemetry::run_span_id(run.as_uuid()),
            );
            async {
                if !view
                    .holds(&mut tx, &hold)
                    .await
                    .map_err(EngineError::Store)?
                {
                    return Ok(Outcome::Fenced);
                }
                if !condition.holds(&mut tx).await.map_err(EngineError::Store)? {
                    return Ok(Outcome::Skipped);
                }
                let version = tx.parts().1.version();
                match self
                    .engine
                    .run_addressed(&mut tx, Vec::new(), effects, command)
                    .await
                {
                    Ok(()) => match tx.commit().await.map_err(EngineError::Store) {
                        Ok(()) => Ok(Outcome::Done),
                        Err(error) if content_quota(&error) => {
                            self.stop_for_storage_quota(
                                view,
                                &hold,
                                RunRef {
                                    tenant_id: tenant,
                                    run_id: run,
                                },
                                kind,
                                condition,
                                version,
                            )
                            .await
                        }
                        Err(error) => Err(error),
                    },
                    Err(EngineError::Execution(error))
                        if !matches!(error, ExecutionError::Invariant(_)) =>
                    {
                        Ok(Outcome::Skipped)
                    }
                    Err(error) if content_quota(&error) => {
                        drop(tx);
                        self.stop_for_storage_quota(
                            view,
                            &hold,
                            RunRef {
                                tenant_id: tenant,
                                run_id: run,
                            },
                            kind,
                            condition,
                            version,
                        )
                        .await
                    }
                    Err(other) => Err(other),
                }
            }
            .instrument(span)
            .await
        }
        .await;
        let outcome = outcome.unwrap_or_else(|error: EngineError| {
            tracing::error!(%error, %run, kind, "a scheduler action failed");
            Outcome::Failed
        });
        metrics::counter!("scheduler_actions_total", "kind" => kind, "outcome" => outcome.label())
            .increment(1);
        if outcome == Outcome::Fenced {
            metrics::counter!("scheduler_fenced_total", "kind" => kind).increment(1);
        }
        outcome
    }

    async fn stop_for_storage_quota(
        &self,
        view: &View,
        hold: &Hold,
        RunRef {
            tenant_id: tenant,
            run_id: run,
        }: RunRef,
        kind: &'static str,
        condition: Condition<'_>,
        version: u64,
    ) -> Result<Outcome, EngineError> {
        let Some(mut tx) = self
            .store
            .begin(tenant, run, Lock::Wait)
            .await
            .map_err(EngineError::Store)?
        else {
            return Ok(Outcome::Skipped);
        };
        let shard_held = view
            .holds(&mut tx, hold)
            .await
            .map_err(EngineError::Store)?;
        if let Err(outcome) = recovery_guard(shard_held, version, tx.parts().1.version()) {
            return Ok(outcome);
        }
        if !condition.holds(&mut tx).await.map_err(EngineError::Store)? {
            return Ok(Outcome::Skipped);
        }
        let mut effects = self.effects();
        if tx.parts().1.stop_reason().is_none() {
            effects.audit = Some(AuditRecord::engine(
                tenant,
                AuditAction::RunStorageQuotaExceeded,
                ResourceType::Run,
                run.to_string(),
                "storage quota exceeded",
                tx.info().trace_id.clone(),
                tx.now(),
            ));
        }
        self.engine
            .run(&mut tx, effects, |def, state, now| {
                Ok(((), execution::stop_for_storage_quota(def, state, now)?))
            })
            .await?;
        tx.commit().await.map_err(EngineError::Store)?;
        metrics::counter!(
            "scheduler_quota_stops_total",
            "limit" => sagat_domain::quota::QuotaResource::TenantContentBytes.limit_name(),
            "kind" => kind,
        )
        .increment(1);
        Ok(Outcome::QuotaStopped)
    }

    async fn stop_loop_for_expression(
        &self,
        view: &View,
        hold: &Hold,
        run: RunRef,
        version: u64,
    ) -> Result<Outcome, EngineError> {
        self.stop_with_code(view, hold, run, version, FailureCode::expression_error())
            .await
    }

    /// Fails the run with a declared hard budget's durable code, counting the refusal under the
    /// closed `workflow_primitive_rejections_total{kind,code}` labels.
    async fn stop_for_limit(
        &self,
        view: &View,
        hold: &Hold,
        run: RunRef,
        kind: WorkflowPrimitiveKind,
        code: FailureCode,
        version: u64,
    ) -> Result<Outcome, EngineError> {
        let rejection = WorkflowPrimitiveRejectionCode::from_reason(code.as_str());
        let outcome = self.stop_with_code(view, hold, run, version, code).await?;
        if outcome == Outcome::Done
            && let Some(rejection) = rejection
        {
            metrics::counter!(
                WorkflowPrimitiveMetric::Rejections.as_str(),
                "kind" => kind.as_str(),
                "code" => rejection.as_str(),
            )
            .increment(1);
        }
        Ok(outcome)
    }

    async fn stop_with_code(
        &self,
        view: &View,
        hold: &Hold,
        RunRef {
            tenant_id: tenant,
            run_id: run,
        }: RunRef,
        version: u64,
        code: FailureCode,
    ) -> Result<Outcome, EngineError> {
        let Some(mut tx) = self
            .store
            .begin(tenant, run, Lock::Wait)
            .await
            .map_err(EngineError::Store)?
        else {
            return Ok(Outcome::Skipped);
        };
        let shard_held = view
            .holds(&mut tx, hold)
            .await
            .map_err(EngineError::Store)?;
        if let Err(outcome) = recovery_guard(shard_held, version, tx.parts().1.version()) {
            return Ok(outcome);
        }
        self.engine
            .run(&mut tx, self.effects(), |definition, state, now| {
                Ok((
                    (),
                    execution::stop_run(definition, state, StopReason::Failed, Some(code), now)?,
                ))
            })
            .await?;
        tx.commit().await.map_err(EngineError::Store)?;
        Ok(Outcome::Done)
    }

    /// [`Self::apply_if`] with nothing to re-check: the domain command itself refuses what is stale.
    async fn apply<F>(
        &self,
        view: &View,
        tenant: TenantId,
        run: RunId,
        kind: &'static str,
        effects: Effects,
        command: F,
    ) -> Outcome
    where
        F: FnOnce(
            &sagat_domain::workflow::WorkflowDefinition,
            &mut sagat_domain::execution::RunState,
            OffsetDateTime,
        ) -> Result<((), Progress), ExecutionError>,
    {
        self.apply_if(
            view,
            RunRef {
                tenant_id: tenant,
                run_id: run,
            },
            kind,
            Condition::Always,
            effects,
            command,
        )
        .await
    }

    async fn apply_addressed<F>(
        &self,
        view: &View,
        tenant: TenantId,
        run: RunId,
        kind: &'static str,
        effects: Effects,
        command: F,
    ) -> Outcome
    where
        F: FnOnce(
            &sagat_domain::workflow::WorkflowDefinition,
            &mut sagat_domain::execution::RunState,
            OffsetDateTime,
        ) -> Result<((), AddressedProgress), ExecutionError>,
    {
        self.apply_addressed_if(
            view,
            RunRef {
                tenant_id: tenant,
                run_id: run,
            },
            kind,
            Condition::Always,
            effects,
            command,
        )
        .await
    }

    fn effects(&self) -> Effects {
        Effects::by(Actor::System("scheduler"), None)
    }

    fn tally(report: &mut Report, outcome: Outcome, done: impl FnOnce(&mut Report)) {
        match outcome {
            Outcome::Done => done(report),
            Outcome::Skipped => report.skipped += 1,
            Outcome::Failed => report.failed += 1,
            Outcome::Fenced => report.fenced += 1,
            Outcome::QuotaStopped => report.quota_stops += 1,
        }
    }

    /// Reads the candidates of a pass: the oldest, taken fairly between the tenants (`FR-010-002`),
    /// from the shards this replica holds. A query that fails is logged and the pass has nothing to
    /// do this tick.
    async fn pick<'a, T, K, Fut>(
        &'a self,
        view: &View,
        what: &'static str,
        report: &mut Report,
        tenant: impl Fn(&T) -> TenantId,
        key: impl Fn(&T) -> K,
        query: impl Fn(&'a S, Window) -> Fut,
    ) -> Vec<T>
    where
        K: Eq + Hash,
        Fut: Future<Output = Result<Vec<T>, StoreError>>,
    {
        let (shards, limit, store) = (view.shards, self.settings.batch, &self.store);
        if !self.settings.fair {
            let oldest = query(
                store,
                Window {
                    shards,
                    exclude: Vec::new(),
                    limit,
                },
            )
            .await;
            return oldest.unwrap_or_else(|error| {
                tracing::warn!(%error, what, "a scheduler query failed");
                report.failed += 1;
                Vec::new()
            });
        }
        let now = Instant::now();
        let discover = self
            .rests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(what)
            .is_none_or(|until| *until <= now);
        let selected = fairness::select(limit, discover, &view.weights, tenant, key, |exclude| {
            query(
                store,
                Window {
                    shards,
                    exclude,
                    limit,
                },
            )
        })
        .await;
        if discover && let Ok((_, selection)) = &selected {
            let rest = fairness::rest_after(self.settings.discovery_rest, selection);
            if !rest.is_zero() {
                self.rests
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(what, Instant::now() + rest);
            }
        }
        match selected {
            Ok((items, selection)) => {
                if selection.limited > 0 {
                    metrics::counter!("scheduler_fairness_limited_total", "pass" => what)
                        .increment(selection.limited as u64);
                }
                if selection.exhausted {
                    metrics::counter!("scheduler_fairness_exhausted_total", "pass" => what)
                        .increment(1);
                }
                items
            }
            Err(error) => {
                tracing::warn!(%error, what, "a scheduler query failed");
                report.failed += 1;
                Vec::new()
            }
        }
    }

    /// Acts on candidates, [`SchedulerSettings::concurrency`] at a time.
    async fn each<T, O, Fut>(&self, items: Vec<T>, work: impl FnMut(T) -> Fut) -> Vec<O>
    where
        Fut: Future<Output = O>,
    {
        stream::iter(items)
            .map(work)
            .buffer_unordered(self.settings.concurrency.max(1))
            .collect()
            .await
    }

    async fn collections(&self, view: &View, report: &mut Report) {
        if view.snapshot.is_none() {
            return;
        }
        let candidates = self
            .pick(
                view,
                "collections",
                report,
                |step: &StepRef| step.tenant_id,
                |step| (step.run_id, step.instance()),
                |store, window| async move { store.collection_candidates(&window).await },
            )
            .await;
        let outcomes = self
            .each(candidates, |candidate| async move {
                let run = candidate.run_id;
                let instance = candidate.instance();
                let outcome = match self
                    .advance_collection_in_view(
                        view,
                        candidate.tenant_id,
                        candidate.run_id,
                        &instance,
                    )
                    .await
                {
                    Ok(
                        CollectionAdvance::Entered
                        | CollectionAdvance::Admitted { .. }
                        | CollectionAdvance::Checkpointed { .. }
                        | CollectionAdvance::DrainStarted
                        | CollectionAdvance::Drained { .. }
                        | CollectionAdvance::DrainFinished
                        | CollectionAdvance::Compensated
                        | CollectionAdvance::Finished,
                    ) => Outcome::Done,
                    Ok(CollectionAdvance::QuotaStopped) => Outcome::QuotaStopped,
                    Ok(CollectionAdvance::LimitStopped) => Outcome::Done,
                    Ok(CollectionAdvance::Waiting | CollectionAdvance::AlreadyFinished)
                    | Err(CollectionAdvanceError::NotCollection)
                    | Err(CollectionAdvanceError::Primitive(
                        PrimitiveStoreError::AtCapacity
                        | PrimitiveStoreError::RevisionConflict { .. },
                    )) => Outcome::Skipped,
                    Err(CollectionAdvanceError::FenceUnavailable)
                    | Err(CollectionAdvanceError::Primitive(
                        PrimitiveStoreError::SchedulerFenceLost,
                    )) => Outcome::Fenced,
                    Err(error) => {
                        tracing::error!(%error, %run, ?instance, "a collection scheduler action failed");
                        Outcome::Failed
                    }
                };
                metrics::counter!(
                    "scheduler_actions_total",
                    "kind" => "collection",
                    "outcome" => outcome.label()
                )
                .increment(1);
                if outcome == Outcome::Fenced {
                    metrics::counter!("scheduler_fenced_total", "kind" => "collection")
                        .increment(1);
                }
                outcome
            })
            .await;
        for outcome in outcomes {
            Self::tally(report, outcome, |value| value.collections_advanced += 1);
        }
    }

    async fn loops(&self, view: &View, report: &mut Report) {
        if view.snapshot.is_none() {
            return;
        }
        let candidates = self
            .pick(
                view,
                "loops",
                report,
                |candidate: &LoopCandidate| candidate.step.tenant_id,
                |candidate| (candidate.step.run_id, candidate.step.instance()),
                |store, window| async move {
                    store
                        .loop_candidates(&window)
                        .await
                        .map_err(loop_discovery_error)
                },
            )
            .await;
        let outcomes = self
            .each(candidates, |candidate| async move {
                let run = candidate.step.run_id;
                let instance = candidate.step.instance();
                let outcome = match self
                    .advance_loop_in_view(
                        view,
                        candidate.step.tenant_id,
                        run,
                        &instance,
                        candidate.timer_id,
                    )
                    .await
                {
                    Ok(
                        LoopAdvance::Entered
                        | LoopAdvance::Checkpointed { .. }
                        | LoopAdvance::DelayElapsed { .. }
                        | LoopAdvance::DrainStarted
                        | LoopAdvance::Drained { .. }
                        | LoopAdvance::DrainFinished
                        | LoopAdvance::Compensated
                        | LoopAdvance::ExpressionStopped,
                    ) => Outcome::Done,
                    Ok(LoopAdvance::QuotaStopped) => Outcome::QuotaStopped,
                    Ok(LoopAdvance::LimitStopped) => Outcome::Done,
                    Ok(LoopAdvance::Waiting | LoopAdvance::AlreadyFinished)
                    | Err(LoopAdvanceError::NotLoop)
                    | Err(LoopAdvanceError::Progress(
                        LoopProgressError::AtCapacity | LoopProgressError::RevisionConflict { .. },
                    )) => Outcome::Skipped,
                    Err(LoopAdvanceError::FenceUnavailable)
                    | Err(LoopAdvanceError::Progress(LoopProgressError::SchedulerFenceLost)) => {
                        Outcome::Fenced
                    }
                    Err(error) => {
                        tracing::error!(%error, %run, ?instance, "a loop scheduler action failed");
                        Outcome::Failed
                    }
                };
                metrics::counter!(
                    "scheduler_actions_total",
                    "kind" => "loop",
                    "outcome" => outcome.label()
                )
                .increment(1);
                if outcome == Outcome::Fenced {
                    metrics::counter!("scheduler_fenced_total", "kind" => "loop").increment(1);
                }
                outcome
            })
            .await;
        for outcome in outcomes {
            Self::tally(report, outcome, |value| value.loops_advanced += 1);
        }
    }

    async fn continuations(&self, view: &View, report: &mut Report)
    where
        S: ChainCommandStore + ChainProgressStore,
        S::Tx: ChainRunTx,
    {
        if view.snapshot.is_none() {
            return;
        }
        let candidates = self
            .pick(
                view,
                "continuations",
                report,
                |candidate: &PendingContinuation| candidate.tenant_id,
                |candidate| (candidate.chain_id, candidate.run_id, candidate.node.clone()),
                |store, window| async move { store.continuation_candidates(&window).await },
            )
            .await;
        let outcomes = self
            .each(candidates, |candidate| async move {
                let outcome = match self
                    .advance_continuation_in_view(
                        view,
                        candidate.tenant_id,
                        candidate.namespace_id,
                        candidate.chain_id,
                        &candidate.node,
                    )
                    .await
                {
                    Ok(_)
                    | Err(
                        ContinuationAdvanceError::ChainLimitFinalized
                        | ContinuationAdvanceError::ExpressionFinalized,
                    ) => Outcome::Done,
                    Err(
                        ContinuationAdvanceError::MissingChain
                        | ContinuationAdvanceError::NotContinuation
                        | ContinuationAdvanceError::Progress(ChainProgressError::Chain(
                            sagat_domain::execution::ChainError::ContinuationConflict,
                        )),
                    ) => Outcome::Skipped,
                    Err(ContinuationAdvanceError::FenceUnavailable)
                    | Err(ContinuationAdvanceError::Progress(
                        ChainProgressError::SchedulerFenceLost,
                    )) => Outcome::Fenced,
                    Err(error) => {
                        tracing::error!(
                            %error,
                            run_id = %candidate.run_id,
                            chain_id = %candidate.chain_id,
                            node = %candidate.node,
                            "a continuation scheduler action failed"
                        );
                        Outcome::Failed
                    }
                };
                metrics::counter!(
                    "scheduler_actions_total",
                    "kind" => "continuation",
                    "outcome" => outcome.label()
                )
                .increment(1);
                if outcome == Outcome::Fenced {
                    metrics::counter!("scheduler_fenced_total", "kind" => "continuation")
                        .increment(1);
                }
                outcome
            })
            .await;
        for outcome in outcomes {
            Self::tally(report, outcome, |value| value.continuations_advanced += 1);
        }
    }

    async fn approvals_open(&self, view: &View, report: &mut Report) {
        if view.snapshot.is_none() {
            return;
        }
        let candidates = self
            .pick(
                view,
                "approval_open",
                report,
                |step: &StepRef| step.tenant_id,
                |step| (step.run_id, step.instance()),
                |store, window| async move { store.approval_candidates(&window).await },
            )
            .await;
        let outcomes = self
            .each(candidates, |candidate| {
                let approvals = ApprovalScheduler::new(&self.store);
                async move {
                    let run = candidate.run_id;
                    let instance = candidate.instance();
                    let outcome = match view.hold_for(run) {
                        Hold::Lease(lease) => {
                            let authority = SchedulerAuthority::new(view.owner.clone(), lease);
                            match approvals
                                .open(candidate.tenant_id, run, &instance, authority)
                                .await
                            {
                                Ok(ApprovalAdvance::Opened { .. }) => Outcome::Done,
                                Ok(ApprovalAdvance::Skipped) | Err(ApprovalAdvanceError::NotApproval) => {
                                    Outcome::Skipped
                                }
                                Ok(ApprovalAdvance::Expired) => Outcome::Failed,
                                Err(error) if error.scheduler_fence_lost() => Outcome::Fenced,
                                Err(error) => {
                                    tracing::error!(%error, %run, ?instance, "an approval open action failed");
                                    Outcome::Failed
                                }
                            }
                        }
                        Hold::Lost => Outcome::Fenced,
                        Hold::Nothing => Outcome::Skipped,
                    };
                    metrics::counter!(
                        "scheduler_actions_total",
                        "kind" => "approval_open",
                        "outcome" => outcome.label()
                    )
                    .increment(1);
                    if outcome == Outcome::Fenced {
                        metrics::counter!("scheduler_fenced_total", "kind" => "approval_open")
                            .increment(1);
                    }
                    outcome
                }
            })
            .await;
        for outcome in outcomes {
            Self::tally(report, outcome, |value| value.approvals_opened += 1);
        }
    }

    async fn approvals_expire(&self, view: &View, report: &mut Report) {
        if view.snapshot.is_none() {
            return;
        }
        let candidates = self
            .pick(
                view,
                "approval_expire",
                report,
                |approval: &DueApproval| approval.address().tenant_id(),
                |approval| (approval.run_id(), approval.address().request_id()),
                |store, window| async move { store.due_approvals(&window).await },
            )
            .await;
        let outcomes = self
            .each(candidates, |candidate| {
                let approvals = ApprovalScheduler::new(&self.store);
                async move {
                    let run = candidate.run_id();
                    let outcome = match view.hold_for(run) {
                        Hold::Lease(lease) => {
                            let authority = SchedulerAuthority::new(view.owner.clone(), lease);
                            match approvals.expire(&candidate, authority).await {
                                Ok(ApprovalAdvance::Expired) => Outcome::Done,
                                Ok(ApprovalAdvance::Skipped) => Outcome::Skipped,
                                Ok(ApprovalAdvance::Opened { .. }) => Outcome::Failed,
                                Err(error) if error.scheduler_fence_lost() => Outcome::Fenced,
                                Err(error) => {
                                    tracing::error!(%error, %run, "an approval expiry action failed");
                                    Outcome::Failed
                                }
                            }
                        }
                        Hold::Lost => Outcome::Fenced,
                        Hold::Nothing => Outcome::Skipped,
                    };
                    metrics::counter!(
                        "scheduler_actions_total",
                        "kind" => "approval_expire",
                        "outcome" => outcome.label()
                    )
                    .increment(1);
                    if outcome == Outcome::Fenced {
                        metrics::counter!("scheduler_fenced_total", "kind" => "approval_expire")
                            .increment(1);
                    }
                    outcome
                }
            })
            .await;
        for outcome in outcomes {
            Self::tally(report, outcome, |value| value.approvals_expired += 1);
        }
    }

    /// Continues the runs whose approval has an outcome: the transaction that settled the request
    /// activated the chosen route and committed; this pass releases it, once, under the shard lease.
    async fn approval_routes(&self, view: &View, report: &mut Report) {
        if view.snapshot.is_none() {
            return;
        }
        let pending = self
            .pick(
                view,
                "approval_routes",
                report,
                |step: &StepRef| step.tenant_id,
                |step| (step.run_id, step.instance()),
                |store, window| async move { store.approval_routes(&window).await },
            )
            .await;
        let outcomes = self
            .each(
                pending,
                |StepRef {
                     tenant_id, run_id, ..
                 }| {
                    self.apply(
                        view,
                        tenant_id,
                        run_id,
                        "approval_route",
                        self.effects(),
                        |def, state, now| Ok(((), execution::release_pending(def, state, now)?)),
                    )
                },
            )
            .await;
        for outcome in outcomes {
            Self::tally(report, outcome, |r| r.approval_routes_released += 1);
        }
    }

    async fn timers(&self, view: &View, report: &mut Report) {
        let due = self
            .pick(
                view,
                "timers",
                report,
                |s: &StepRef| s.tenant_id,
                |s| (s.run_id, s.node.clone()),
                |store, window| async move { store.due_timers(&window).await },
            )
            .await;
        let outcomes = self
            .each(
                due,
                |StepRef {
                     tenant_id,
                     run_id,
                     node,
                     ..
                 }| {
                    self.apply(
                        view,
                        tenant_id,
                        run_id,
                        "timer",
                        self.effects(),
                        move |def, state, now| {
                            Ok(((), execution::fire_timer(def, state, &node, now)?))
                        },
                    )
                },
            )
            .await;
        for outcome in outcomes {
            Self::tally(report, outcome, |r| r.timers_fired += 1);
        }
    }

    async fn wait_deadlines(&self, view: &View, report: &mut Report) {
        let due = self
            .pick(
                view,
                "wait_deadlines",
                report,
                |s: &StepRef| s.tenant_id,
                |s| (s.run_id, s.node.clone()),
                |store, window| async move { store.due_wait_deadlines(&window).await },
            )
            .await;
        let outcomes = self
            .each(
                due,
                |StepRef {
                     tenant_id,
                     run_id,
                     node,
                     ..
                 }| {
                    self.apply(
                        view,
                        tenant_id,
                        run_id,
                        "wait_timeout",
                        self.effects(),
                        move |def, state, now| {
                            Ok(((), execution::time_out_wait(def, state, &node, now)?))
                        },
                    )
                },
            )
            .await;
        for outcome in outcomes {
            Self::tally(report, outcome, |r| r.waits_timed_out += 1);
        }
    }

    async fn retries(&self, view: &View, report: &mut Report) {
        let due = self
            .pick(
                view,
                "retries",
                report,
                |s: &StepRef| s.tenant_id,
                |s| (s.run_id, s.instance()),
                |store, window| async move { store.due_retries(&window).await },
            )
            .await;
        let outcomes = self
            .each(
                due,
                |StepRef {
                     tenant_id,
                     run_id,
                     node,
                     instance_path,
                 }| {
                    async move {
                        let instance = ExecutionInstanceId::in_path(instance_path, node);
                        if instance.path().is_root() {
                            self.apply(
                                view,
                                tenant_id,
                                run_id,
                                "retry",
                                self.effects(),
                                move |def, state, now| {
                                    Ok((
                                        (),
                                        execution::retry_due(def, state, instance.node(), now)?,
                                    ))
                                },
                            )
                            .await
                        } else {
                            self.apply_addressed(
                                view,
                                tenant_id,
                                run_id,
                                "retry",
                                self.effects(),
                                move |def, state, now| {
                                    Ok((
                                        (),
                                        execution::retry_due_instance(def, state, &instance, now)?,
                                    ))
                                },
                            )
                            .await
                        }
                    }
                },
            )
            .await;
        for outcome in outcomes {
            Self::tally(report, outcome, |r| r.retries_released += 1);
        }
    }

    async fn leases(&self, view: &View, report: &mut Report) {
        let lost = self
            .pick(
                view,
                "leases",
                report,
                |l: &sagat_application::maintenance::LostLease| l.step.tenant_id,
                |l| (l.step.run_id, l.step.instance()),
                |store, window| async move { store.lost_leases(&window).await },
            )
            .await;
        let outcomes = self
            .each(lost, |lost| async move {
                let jitter = self.entropy.unit();
                let cause = match lost.cause {
                    LeaseLoss::Expired => Expiry::LeaseExpired,
                    LeaseLoss::TimedOut => Expiry::AttemptTimeout,
                };
                let fence = lost.fence;
                let instance = lost.step.instance();
                let command_instance = instance.clone();
                let run = RunRef {
                    tenant_id: lost.step.tenant_id,
                    run_id: lost.step.run_id,
                };
                let condition = Condition::LeaseLost {
                    instance: &instance,
                    cause: lost.cause,
                };
                if instance.path().is_root() {
                    self.apply_if(
                        view,
                        run,
                        "lease",
                        condition,
                        self.effects(),
                        move |def, state, now| {
                            Ok((
                                (),
                                execution::expire_attempt(
                                    def,
                                    state,
                                    command_instance.node(),
                                    fence,
                                    cause,
                                    jitter,
                                    now,
                                )?,
                            ))
                        },
                    )
                    .await
                } else {
                    self.apply_addressed_if(
                        view,
                        run,
                        "lease",
                        condition,
                        self.effects(),
                        move |def, state, now| {
                            Ok((
                                (),
                                execution::expire_instance(
                                    def,
                                    state,
                                    &command_instance,
                                    fence,
                                    cause,
                                    jitter,
                                    now,
                                )?,
                            ))
                        },
                    )
                    .await
                }
            })
            .await;
        for outcome in outcomes {
            Self::tally(report, outcome, |r| r.leases_expired += 1);
        }
    }

    async fn deadlines(&self, view: &View, report: &mut Report) {
        let overdue = self
            .pick(
                view,
                "deadlines",
                report,
                |r: &RunRef| r.tenant_id,
                |r| r.run_id,
                |store, window| async move { store.overdue_runs(&window).await },
            )
            .await;
        let outcomes = self
            .each(overdue, |RunRef { tenant_id, run_id }| {
                self.apply(
                    view,
                    tenant_id,
                    run_id,
                    "deadline",
                    self.effects(),
                    |def, state, now| {
                        Ok((
                            (),
                            execution::stop_run(def, state, StopReason::TimedOut, None, now)?,
                        ))
                    },
                )
            })
            .await;
        for outcome in outcomes {
            Self::tally(report, outcome, |r| r.deadlines_enforced += 1);
        }
    }

    /// Ends the runs that a transaction of their own stopped (a loop at its iteration limit): the
    /// planner drains what is left and finishes them, which no worker report or timer would do.
    async fn stopped_runs(&self, view: &View, report: &mut Report) {
        let stopped = self
            .pick(
                view,
                "stopped_runs",
                report,
                |r: &RunRef| r.tenant_id,
                |r| r.run_id,
                |store, window| async move { store.stopped_runs(&window).await },
            )
            .await;
        let outcomes = self
            .each(stopped, |RunRef { tenant_id, run_id }| {
                self.apply(
                    view,
                    tenant_id,
                    run_id,
                    "stopped_run",
                    self.effects(),
                    |def, state, now| Ok(((), execution::release_pending(def, state, now)?)),
                )
            })
            .await;
        for outcome in outcomes {
            Self::tally(report, outcome, |r| r.stopped_runs_settled += 1);
        }
    }

    async fn hints(&self, view: &View, report: &mut Report) {
        let older_than = self.settings.rehint_after;
        let stale = self
            .pick(
                view,
                "hints",
                report,
                |s: &StepRef| s.tenant_id,
                |s| (s.run_id, s.instance()),
                |store, window| async move { store.stale_hints(older_than, &window).await },
            )
            .await;
        let outcomes = self
            .each(
                stale,
                |StepRef {
                     tenant_id,
                     run_id,
                     node,
                     instance_path,
                 }| async move {
                    let mut effects = self.effects();
                    if instance_path.is_root() {
                        effects.rehint = vec![node];
                        self.apply(view, tenant_id, run_id, "rehint", effects, |_, _, _| {
                            Ok((
                                (),
                                Progress {
                                    events: Vec::new(),
                                    pending: None,
                                },
                            ))
                        })
                        .await
                    } else {
                        effects.rehint_instances =
                            vec![ExecutionInstanceId::in_path(instance_path, node)];
                        self.apply_addressed(
                            view,
                            tenant_id,
                            run_id,
                            "rehint",
                            effects,
                            |_, _, _| Ok(((), AddressedProgress { events: Vec::new() })),
                        )
                        .await
                    }
                },
            )
            .await;
        for outcome in outcomes {
            Self::tally(report, outcome, |r| r.hints_renewed += 1);
        }
    }

    // ------------------------------------------------------------------ child runs (FR-002-009)

    /// Creates the child runs that `child` steps reserved. Each is created, started and its
    /// reservation claimed in one transaction under the parent's lock, so a child exists exactly
    /// when its parent step was still waiting for it.
    async fn children_start(&self, view: &View, report: &mut Report) {
        let Some(definitions) = &self.definitions else {
            return;
        };
        let pending = self
            .pick(
                view,
                "pending_children",
                report,
                |c: &PendingChild| c.step.tenant_id,
                |c| (c.step.run_id, c.step.node.clone()),
                |store, window| async move { store.pending_children(&window).await },
            )
            .await;
        let outcomes = self
            .each(pending, |child| async move {
                let (outcome, counted) = match self.start_child(view, definitions, &child).await {
                    Ok(done) => done,
                    Err(error) => {
                        tracing::error!(%error, run = %child.step.run_id, node = %child.step.node, "starting a child run failed");
                        (Outcome::Failed, false)
                    }
                };
                if !counted {
                    metrics::counter!("scheduler_actions_total", "kind" => "child_start", "outcome" => outcome.label())
                        .increment(1);
                    if outcome == Outcome::Fenced {
                        metrics::counter!("scheduler_fenced_total", "kind" => "child_start")
                            .increment(1);
                    }
                }
                (outcome, counted)
            })
            .await;
        for (outcome, told_the_parent) in outcomes {
            if told_the_parent {
                Self::tally(report, outcome, |r| r.children_finished += 1);
            } else {
                Self::tally(report, outcome, |r| r.children_started += 1);
            }
        }
    }

    /// Starts the child a step reserved. The flag says that the answer is the parent's step being told
    /// that the child cannot be (the action was counted, as a `child_finish`, when it was applied).
    async fn start_child(
        &self,
        view: &View,
        definitions: &DefinitionCache,
        pending: &PendingChild,
    ) -> Result<(Outcome, bool), EngineError> {
        let tenant = pending.step.tenant_id;
        let parent_id = pending.step.run_id;
        let node = &pending.step.node;
        // The shard is the parent's: it is the parent step that waits for this child, and the
        // candidate was found through it. (The child's own shard is whoever owns the child's id.)
        let hold = view.hold_for(parent_id);
        if matches!(hold, Hold::Lost) {
            return Ok((Outcome::Fenced, false));
        }
        let Some(parent) = self.store.get(tenant, parent_id).await? else {
            return Ok((Outcome::Skipped, false));
        };
        let Some(parent_version) = self
            .store
            .get_version_by_id(tenant, parent.info.version_id)
            .await?
        else {
            return Ok((Outcome::Skipped, false));
        };
        let parent_definition = definitions
            .get_or_parse(parent.info.version_id, &parent_version.document)
            .map_err(|_| StoreError::Internal {
                detail: "a stored version no longer parses".into(),
            })?;
        let Some(parent_node) = parent_definition.node(node) else {
            return Ok((Outcome::Skipped, false));
        };
        let NodeKind::Child {
            workflow,
            version,
            cancel_policy,
            ..
        } = parent_node.kind()
        else {
            return Ok((Outcome::Skipped, false));
        };

        // The child workflow lives in the parent's namespace; the lookup is scoped by tenant and
        // namespace, so a same-named workflow elsewhere can never be reached.
        let target = match self
            .store
            .find_workflow_by_name(tenant, parent.info.namespace_id, workflow)
            .await?
        {
            Some(record) => {
                let version_record = match version {
                    Some(number) => {
                        self.store
                            .get_version(tenant, record.workflow.id(), *number)
                            .await?
                    }
                    None => match record.workflow.active_version() {
                        Some(id) => self.store.get_version_by_id(tenant, id).await?,
                        None => None,
                    },
                };
                version_record.map(|v| (record, v))
            }
            None => None,
        };
        let Some((workflow_record, version_record)) = target else {
            // The child cannot exist: tell the parent step now instead of waiting forever.
            return Ok((
                self.finish_with(
                    view,
                    tenant,
                    parent_id,
                    node.clone(),
                    pending.child,
                    ChildOutcome::Failed(FailureCode::child_not_found()),
                )
                .await,
                true,
            ));
        };
        let definition = definitions
            .get_or_parse(version_record.version.id(), &version_record.document)
            .map_err(|_| StoreError::Internal {
                detail: "a stored version no longer parses".into(),
            })?;

        // A child never outlives its parent's deadline, and its own timeout can only shorten it.
        let own = parent_node
            .timeout()
            .and_then(|t| time::Duration::try_from(t).ok())
            .and_then(|t| pending.as_of.checked_add(t));
        let deadline_at = match (parent.info.deadline_at, own) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let child_id = pending.child;
        let info = RunInfo {
            run_id: child_id,
            tenant_id: tenant,
            namespace_id: parent.info.namespace_id,
            workflow_id: workflow_record.workflow.id(),
            version_id: version_record.version.id(),
            started_by: sagat_domain::identity::Subject::engine()
                .as_str()
                .to_owned(),
            correlation_id: parent.info.correlation_id.clone(),
            trace_id: parent.info.trace_id.clone(),
            parent_span: None,
            rerun_of: None,
            deadline_at,
            created_at: pending.as_of,
            parent: Some(ParentLink {
                run: parent_id,
                node: node.clone(),
                cancel_policy: *cancel_policy,
            }),
            depth: parent.info.depth.saturating_add(1),
            root: parent.info.root,
        };
        let audit = AuditRecord::engine(
            tenant,
            AuditAction::RunStarted,
            ResourceType::Run,
            child_id.to_string(),
            format!("child of run {parent_id}, node {node}"),
            parent.info.trace_id.clone(),
            pending.as_of,
        );
        let limits = RunLimits::of(&self.store, tenant, parent.info.namespace_id).await?;
        let new = NewRun {
            limits,
            info,
            definition: std::sync::Arc::clone(&definition),
            input: pending.input.clone(),
            input_blob: Some(sagat_application::blobs::RunInputBinding::Inherit(
                parent_id,
            )),
            audit,
            idempotency: Idempotency {
                scope: format!("child:{parent_id}"),
                // (The dedupe table requires 8 to 160 characters, like an `Idempotency-Key`.)
                key: format!("child-start:{node}"),
                fingerprint: child_id.as_uuid().as_bytes().to_vec(),
            },
        };
        match self.store.create(new).await {
            Ok(Applied::Replayed(_)) => Ok((Outcome::Skipped, false)),
            // The parent step no longer waits for this run (cancelled, or created by a peer).
            Err(StoreError::MissingReference) => Ok((Outcome::Skipped, false)),
            // Either way the step fails instead of waiting. Waiting for room would leave the
            // blocked reservations at the head of the oldest-first queue, where a full namespace of
            // one tenant would keep every other tenant's children from starting (G32, feature
            // 010); the failure code lets the workflow tell the two apart and route on it.
            Err(StoreError::QuotaExceeded { what }) => Ok((
                {
                    let code = if what == sagat_application::execution::TREE_LIMIT {
                        // The tree is as large as a tree may get.
                        FailureCode::child_limit_exceeded()
                    } else {
                        // The namespace or the tenant is at its quota of active runs.
                        FailureCode::child_quota_exceeded()
                    };
                    self.finish_with(
                        view,
                        tenant,
                        parent_id,
                        node.clone(),
                        pending.child,
                        ChildOutcome::Failed(code),
                    )
                    .await
                },
                true,
            )),
            Err(other) => Err(other.into()),
            Ok(Applied::Done(mut tx)) => {
                // The fence, before anything is committed: a replica that lost the parent's shard
                // while it was reading must not leave a child run behind (dropping rolls it back).
                if !view.holds(&mut tx, &hold).await? {
                    return Ok((Outcome::Fenced, false));
                }
                let effects = Effects::by(Actor::System("scheduler"), parent.info.trace_id.clone());
                let started = self
                    .engine
                    .run(&mut tx, effects, |def: &WorkflowDefinition, state, now| {
                        let (started, progress) = execution::start_run(def, child_id, now)?;
                        *state = started;
                        Ok(((), progress))
                    })
                    .await;
                if let Err(error) = started {
                    drop(tx);
                    if content_quota(&error) {
                        return Ok((
                            self.finish_with(
                                view,
                                tenant,
                                parent_id,
                                node.clone(),
                                pending.child,
                                ChildOutcome::Failed(FailureCode::child_quota_exceeded()),
                            )
                            .await,
                            true,
                        ));
                    }
                    return Err(error);
                }
                if let Err(error) = tx.commit().await {
                    if content_store_quota(&error) {
                        return Ok((
                            self.finish_with(
                                view,
                                tenant,
                                parent_id,
                                node.clone(),
                                pending.child,
                                ChildOutcome::Failed(FailureCode::child_quota_exceeded()),
                            )
                            .await,
                            true,
                        ));
                    }
                    return Err(error.into());
                }
                metrics::counter!("child_runs_started_total").increment(1);
                Ok((Outcome::Done, false))
            }
        }
    }

    /// Reports a child's outcome to the parent step that waits for it.
    async fn finish_with(
        &self,
        view: &View,
        tenant: TenantId,
        parent: RunId,
        node: sagat_domain::workflow::NodeId,
        child: RunId,
        outcome: ChildOutcome,
    ) -> Outcome {
        self.apply(
            view,
            tenant,
            parent,
            "child_finish",
            self.effects(),
            move |def, state, now| {
                Ok((
                    (),
                    execution::finish_child(def, state, &node, child, outcome, now)?,
                ))
            },
        )
        .await
    }

    /// Tells parents that the child they wait for has ended.
    async fn children_finish(&self, view: &View, report: &mut Report) {
        let ended = self
            .pick(
                view,
                "ended_children",
                report,
                |c: &EndedChild| c.tenant_id,
                |c| c.child,
                |store, window| async move { store.ended_children(&window).await },
            )
            .await;
        let outcomes = self
            .each(
                ended,
                |EndedChild {
                     tenant_id,
                     child,
                     parent,
                     node,
                     status,
                 }| async move {
                    // Every way a run can end, and nothing else: a status added later must be mapped
                    // here (a run that ended in a way nobody reports would keep its parent waiting
                    // for ever, and sit at the head of the list of ended children).
                    let outcome = match status {
                        RunStatus::Succeeded => ChildOutcome::Succeeded,
                        RunStatus::Failed => ChildOutcome::Failed(FailureCode::child_failed()),
                        RunStatus::Cancelled => {
                            ChildOutcome::Failed(FailureCode::child_cancelled())
                        }
                        RunStatus::TimedOut => {
                            ChildOutcome::Failed(FailureCode::child_timed_out())
                        }
                        RunStatus::Compensated => {
                            ChildOutcome::Failed(FailureCode::child_compensated())
                        }
                        RunStatus::Partial => ChildOutcome::Failed(FailureCode::child_partial()),
                        RunStatus::CompensationFailed => {
                            ChildOutcome::Failed(FailureCode::child_compensation_failed())
                        }
                        RunStatus::Running
                        | RunStatus::Waiting
                        | RunStatus::Paused
                        | RunStatus::Cancelling
                        | RunStatus::Compensating
                        | RunStatus::ManualReview => return None,
                    };
                    let result = self
                        .finish_with(view, tenant_id, parent, node.clone(), child, outcome)
                        .await;
                    if result == Outcome::Skipped {
                        tracing::warn!(%parent, %node, %child, "an ended child could not be reported to its parent step");
                    }
                    Some(result)
                },
            )
            .await;
        for outcome in outcomes.into_iter().flatten() {
            Self::tally(report, outcome, |r| r.children_finished += 1);
        }
    }

    /// Stops the running children of a parent that was stopped under a policy that cancels them.
    async fn children_cancel(&self, view: &View, report: &mut Report) {
        let doomed = self
            .pick(
                view,
                "children_to_cancel",
                report,
                |r: &RunRef| r.tenant_id,
                |r| r.run_id,
                |store, window| async move { store.children_to_cancel(&window).await },
            )
            .await;
        let outcomes = self
            .each(doomed, |RunRef { tenant_id, run_id }| {
                self.apply(
                    view,
                    tenant_id,
                    run_id,
                    "child_cancel",
                    self.effects(),
                    |def, state, now| {
                        Ok((
                            (),
                            execution::stop_run(def, state, StopReason::Cancelled, None, now)?,
                        ))
                    },
                )
            })
            .await;
        for outcome in outcomes {
            Self::tally(report, outcome, |r| r.children_cancelled += 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use sagat_application::error::StoreError;
    use sagat_application::loop_progress::LoopProgressError;
    use sagat_application::primitives::PrimitiveStoreError;
    use sagat_domain::quota::QuotaResource;

    use super::{Outcome, collection_content_quota, loop_content_quota, recovery_guard};

    #[test]
    fn quota_recovery_does_not_mutate_after_version_change_or_shard_loss() {
        for (held, expected, actual, outcome) in [
            (true, 7, 8, Outcome::Skipped),
            (false, 7, 7, Outcome::Fenced),
            (false, 7, 8, Outcome::Fenced),
        ] {
            let mut mutations = 0;
            let result = recovery_guard(held, expected, actual);
            if result.is_ok() {
                mutations += 1;
            }
            assert_eq!(result, Err(outcome));
            assert_eq!(mutations, 0);
        }
        assert_eq!(recovery_guard(true, 7, 7), Ok(()));
    }

    #[test]
    fn only_the_retained_content_limit_enters_the_terminal_recovery_path() {
        let content = StoreError::QuotaExceeded {
            what: QuotaResource::TenantContentBytes.limit_name(),
        };
        assert!(collection_content_quota(&PrimitiveStoreError::Storage(
            content.clone()
        )));
        assert!(loop_content_quota(&LoopProgressError::Storage(content)));

        let capacity = StoreError::QuotaExceeded {
            what: QuotaResource::IterationConcurrency.limit_name(),
        };
        assert!(!collection_content_quota(&PrimitiveStoreError::Storage(
            capacity.clone()
        )));
        assert!(!loop_content_quota(&LoopProgressError::Storage(capacity)));
        assert!(!collection_content_quota(&PrimitiveStoreError::AtCapacity));
        assert!(!loop_content_quota(&LoopProgressError::AtCapacity));
    }
}
