//! Run use cases: start, read, operate and signal (`FR-002-001`..`FR-002-005`, `FR-002-007`).
//!
//! Every use case receives an authenticated [`Principal`], authorizes before touching storage,
//! derives the tenant from it, and performs each state change through [`Engine::run`] inside a
//! [`RunTx`], so the journal, the projections, the audit record and the idempotency record commit
//! together or not at all.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::execution::{
    self, ExecutionError, Progress, RunEvent, RunId, RunStatus, StepStatus, StopReason, WaitingOn,
};
use sagat_domain::identity::{Namespace, NamespaceId, Permission, Principal, TenantId};
use sagat_domain::workflow::{NodeId, NodeKind, WorkflowDefinition};
use time::OffsetDateTime;

use crate::clock::Clock;
use crate::definitions::{Applied, DefinitionStore, Idempotency};
use crate::error::{AppError, StoreError};
use crate::execution::{
    Actor, DefinitionCache, Effects, Engine, EngineError, ExpressionEvaluator, MAX_PAYLOAD_BYTES,
    RunInfo, RunRecord, RunTx,
};
use crate::identity::{Page, PageRequest, RequestContext};
use crate::quotas::{QuotaLookup, RunLimits};

/// Longest a run may be given to finish.
pub const MAX_RUN_DEADLINE: Duration = Duration::from_secs(366 * 24 * 60 * 60);
/// Longest correlation id accepted at start.
pub const MAX_CORRELATION_ID: usize = 160;

/// How a run is opened for a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lock {
    /// Wait for the run's row lock (API commands, timers).
    Wait,
    /// Give up at once when another transaction holds it (workers polling for tasks).
    SkipLocked,
}

/// A run about to be created.
#[derive(Debug, Clone)]
pub struct NewRun {
    /// The quotas the creation is judged against, atomically with the insert.
    pub limits: RunLimits,
    pub info: RunInfo,
    pub definition: Arc<WorkflowDefinition>,
    pub input: Vec<u8>,
    pub audit: AuditRecord,
    pub idempotency: Idempotency,
}

/// One journal entry as the API shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEntry {
    pub seq: u64,
    pub event_type: String,
    pub node: Option<String>,
    pub attempt: Option<u32>,
    pub actor: String,
    pub trace_id: Option<String>,
    pub occurred_at: OffsetDateTime,
    /// Digest of the payload the event refers to (a step output), hex.
    pub payload_ref: Option<String>,
}

/// One step as the runtime graph shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepView {
    pub node: NodeId,
    pub status: StepStatus,
    pub attempt: u32,
    pub generation: u64,
    pub cancel_requested: bool,
    pub failure_code: Option<String>,
    pub next_attempt_at: Option<OffsetDateTime>,
    pub due_at: Option<OffsetDateTime>,
    pub waiting_on: Option<&'static str>,
    /// The child run a `child` step reserved.
    pub child_run: Option<RunId>,
}

/// A run, its definition and its steps, read at one instant.
#[derive(Debug, Clone)]
pub struct GraphSnapshot {
    pub run: RunRecord,
    pub definition: Arc<WorkflowDefinition>,
    pub steps: Vec<StepView>,
}

/// Filters of a run listing.
#[derive(Debug, Clone, Default)]
pub struct RunFilter {
    pub workflow: Option<WorkflowId>,
    pub status: Option<RunStatus>,
}

/// Storage port for runs. Implemented by `sagat-infra-postgres`.
///
/// Every method takes the tenant explicitly; adapters MUST apply it as a filter on every statement.
pub trait RunStore: QuotaLookup + Send + Sync {
    type Tx: RunTx;

    /// Creates the run (row, input payload, empty state) and returns its open transaction, or
    /// reports that the same request already created one.
    fn create(
        &self,
        new: NewRun,
    ) -> impl Future<Output = Result<Applied<Self::Tx>, StoreError>> + Send;

    /// Opens a run for a command. `None` when it does not exist (or, with
    /// [`Lock::SkipLocked`], when another transaction holds it).
    fn begin(
        &self,
        tenant: TenantId,
        run: RunId,
        lock: Lock,
    ) -> impl Future<Output = Result<Option<Self::Tx>, StoreError>> + Send;

    fn get(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> impl Future<Output = Result<Option<RunRecord>, StoreError>> + Send;

    /// Runs of a namespace newest first, with `id < before` when given.
    fn list(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        filter: &RunFilter,
        before: Option<RunId>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<RunRecord>, StoreError>> + Send;

    /// Journal entries with `seq > after`, ascending. `None` when the run does not exist.
    fn journal(
        &self,
        tenant: TenantId,
        run: RunId,
        after: Option<u64>,
        limit: u32,
    ) -> impl Future<Output = Result<Option<Vec<JournalEntry>>, StoreError>> + Send;

    fn graph(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> impl Future<Output = Result<Option<GraphSnapshot>, StoreError>> + Send;

    /// Whether the run exists in a tenant *other than* `tenant` (decides whether a miss deserves a
    /// cross-tenant audit record; never returned to clients).
    fn run_exists_outside(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Best-effort, deduplicated cross-tenant denial record.
    fn record_denial(
        &self,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}

/// What an operator may ask of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOperation {
    Cancel,
    Pause,
    Resume,
}

impl RunOperation {
    const fn action(self) -> AuditAction {
        match self {
            Self::Cancel => AuditAction::RunCancelRequested,
            Self::Pause => AuditAction::RunPaused,
            Self::Resume => AuditAction::RunResumed,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Cancel => "cancel",
            Self::Pause => "pause",
            Self::Resume => "resume",
        }
    }
}

/// What starting a run needs from the caller.
#[derive(Debug, Clone)]
pub struct StartRun<'a> {
    pub namespace: &'a str,
    pub workflow: WorkflowId,
    /// Pins a version; the workflow's active version otherwise.
    pub version: Option<WorkflowVersionId>,
    pub input: &'a [u8],
    pub correlation_id: Option<&'a str>,
    pub deadline: Option<OffsetDateTime>,
    pub key: &'a str,
}

/// Run use cases.
pub struct RunService<S, C, E> {
    store: S,
    clock: C,
    engine: Engine<E>,
    cache: DefinitionCache,
}

impl<S, C, E> std::fmt::Debug for RunService<S, C, E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunService").finish_non_exhaustive()
    }
}

impl<S, C, E> RunService<S, C, E>
where
    S: RunStore + DefinitionStore,
    C: Clock,
    E: ExpressionEvaluator,
{
    pub const fn new(store: S, clock: C, engine: Engine<E>, cache: DefinitionCache) -> Self {
        Self {
            store,
            clock,
            engine,
            cache,
        }
    }

    /// Starts a run of a published version. The run is durable when this returns: its first steps
    /// exist and workers can claim them.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `run:start`; [`AppError::NotFound`] for an unknown namespace,
    /// workflow or version; [`AppError::StateConflict`] when the workflow has no published
    /// version; [`AppError::NotAllowed`] for a bad input, deadline or correlation id;
    /// [`AppError::IdempotencyKeyReused`].
    pub async fn start(
        &self,
        principal: &Principal,
        request: &StartRun<'_>,
        ctx: &RequestContext,
    ) -> Result<RunRecord, AppError> {
        principal.authorize(Permission::RunStart)?;
        if request.input.len() > MAX_PAYLOAD_BYTES {
            return Err(AppError::NotAllowed("the run input is larger than 256 KiB"));
        }
        if !self.engine.evaluator().is_json_object(request.input) {
            return Err(AppError::NotAllowed("the run input must be a JSON object"));
        }
        if request
            .correlation_id
            .is_some_and(|c| c.len() > MAX_CORRELATION_ID || c.chars().any(char::is_control))
        {
            return Err(AppError::NotAllowed(
                "the correlation id is too long or has control characters",
            ));
        }
        let namespace = self.namespace(principal, request.namespace).await?;
        let workflow = self
            .store
            .get_workflow(principal.tenant_id(), request.workflow)
            .await?
            .filter(|r| r.workflow.namespace_id() == namespace.id())
            .ok_or(AppError::NotFound)?
            .workflow;
        let version_id = match request.version {
            Some(id) => id,
            None => workflow.active_version().ok_or(AppError::StateConflict(
                "the workflow has no published version",
            ))?,
        };
        let version = self
            .store
            .get_version_by_id(principal.tenant_id(), version_id)
            .await?
            .filter(|v| v.version.workflow_id() == workflow.id())
            .ok_or(AppError::NotFound)?;
        let definition = self
            .cache
            .get_or_parse(version.version.id(), &version.document)
            .map_err(|error| {
                tracing::error!(%error, version = %version.version.id(), "a stored version no longer parses");
                AppError::Internal
            })?;

        let now = self.clock.now();
        if let Some(deadline) = request.deadline {
            let in_the_future = deadline > now;
            let within_limit = deadline - now
                <= time::Duration::try_from(MAX_RUN_DEADLINE).unwrap_or(time::Duration::MAX);
            if !in_the_future || !within_limit {
                return Err(AppError::NotAllowed(
                    "the deadline must be in the future and within a year",
                ));
            }
        }
        let run_id = RunId::generate();
        let info = RunInfo {
            run_id,
            tenant_id: principal.tenant_id(),
            namespace_id: namespace.id(),
            workflow_id: workflow.id(),
            version_id: version.version.id(),
            started_by: principal.subject().as_str().to_owned(),
            correlation_id: request.correlation_id.map(str::to_owned),
            trace_id: ctx.trace_id.clone(),
            deadline_at: request.deadline,
            created_at: now,
            parent: None,
            depth: 0,
            root: run_id,
        };
        let audit = AuditRecord::by(
            principal,
            AuditAction::RunStarted,
            ResourceType::Run,
            run_id.to_string(),
            ctx.trace_id.clone(),
            now,
        );
        let idempotency = Idempotency {
            scope: format!("run.start:{}", namespace.id()),
            key: request.key.to_owned(),
            fingerprint: fingerprint(&[
                workflow.id().to_string().as_bytes(),
                version.version.id().to_string().as_bytes(),
                request.input,
                request.correlation_id.unwrap_or_default().as_bytes(),
                request
                    .deadline
                    .map(|d| d.unix_timestamp_nanos().to_be_bytes())
                    .unwrap_or_default()
                    .as_slice(),
            ]),
        };
        let limits = RunLimits::of(&self.store, principal.tenant_id(), namespace.id()).await?;
        let new = NewRun {
            limits,
            info,
            definition: Arc::clone(&definition),
            input: request.input.to_vec(),
            audit,
            idempotency,
        };
        match self.store.create(new).await? {
            Applied::Replayed(existing) => {
                let id = existing.parse::<RunId>().map_err(|_| AppError::Internal)?;
                self.store
                    .get(principal.tenant_id(), id)
                    .await?
                    .ok_or(AppError::Internal)
            }
            Applied::Done(mut tx) => {
                let effects = Effects::by(
                    Actor::Principal(principal.subject().as_str().to_owned()),
                    ctx.trace_id.clone(),
                );
                self.engine
                    .run(&mut tx, effects, |def, state, now| {
                        let (started, progress) = execution::start_run(def, run_id, now)?;
                        *state = started;
                        Ok(((), progress))
                    })
                    .await
                    .map_err(engine_error)?;
                tx.commit().await?;
                self.store
                    .get(principal.tenant_id(), run_id)
                    .await?
                    .ok_or(AppError::Internal)
            }
        }
    }

    /// # Errors
    /// [`AppError::Forbidden`] without `run:read`; [`AppError::NotFound`] when the run is not in the
    /// caller's tenant (a miss on another tenant's run is audited).
    pub async fn get(
        &self,
        principal: &Principal,
        id: RunId,
        ctx: &RequestContext,
    ) -> Result<RunRecord, AppError> {
        principal.authorize(Permission::RunRead)?;
        match self.store.get(principal.tenant_id(), id).await? {
            Some(run) => Ok(run),
            None => Err(self.missing(principal, id, ctx).await),
        }
    }

    /// # Errors
    /// [`AppError::Forbidden`] without `run:read`; [`AppError::NotFound`] for an unknown namespace;
    /// [`AppError::Invalid`] for a bad cursor.
    pub async fn list(
        &self,
        principal: &Principal,
        namespace: &str,
        filter: &RunFilter,
        page: &PageRequest,
    ) -> Result<Page<RunRecord>, AppError> {
        principal.authorize(Permission::RunRead)?;
        let namespace = self.namespace(principal, namespace).await?;
        let before = page
            .cursor
            .as_deref()
            .map(str::parse::<RunId>)
            .transpose()?;
        let limit = page.effective_limit();
        let mut rows = self
            .store
            .list(
                principal.tenant_id(),
                namespace.id(),
                filter,
                before,
                limit + 1,
            )
            .await?;
        let more = rows.len() > limit as usize;
        rows.truncate(limit as usize);
        let next_cursor = more
            .then(|| rows.last().map(|r| r.info.run_id.to_string()))
            .flatten();
        Ok(Page {
            items: rows,
            next_cursor,
        })
    }

    /// # Errors
    /// [`AppError::Forbidden`] without `run:read`; [`AppError::NotFound`]; [`AppError::Invalid`]
    /// for a bad cursor.
    pub async fn journal(
        &self,
        principal: &Principal,
        id: RunId,
        page: &PageRequest,
        ctx: &RequestContext,
    ) -> Result<Page<JournalEntry>, AppError> {
        principal.authorize(Permission::RunRead)?;
        let after = page
            .cursor
            .as_deref()
            .map(|c| c.parse::<u64>().map_err(|_| AppError::NotFound))
            .transpose()?;
        let limit = page.effective_limit();
        let Some(mut rows) = self
            .store
            .journal(principal.tenant_id(), id, after, limit + 1)
            .await?
        else {
            return Err(self.missing(principal, id, ctx).await);
        };
        let more = rows.len() > limit as usize;
        rows.truncate(limit as usize);
        let next_cursor = more
            .then(|| rows.last().map(|e| e.seq.to_string()))
            .flatten();
        Ok(Page {
            items: rows,
            next_cursor,
        })
    }

    /// # Errors
    /// [`AppError::Forbidden`] without `run:read`; [`AppError::NotFound`].
    pub async fn graph(
        &self,
        principal: &Principal,
        id: RunId,
        ctx: &RequestContext,
    ) -> Result<GraphSnapshot, AppError> {
        principal.authorize(Permission::RunRead)?;
        match self.store.graph(principal.tenant_id(), id).await? {
            Some(graph) => Ok(graph),
            None => Err(self.missing(principal, id, ctx).await),
        }
    }

    /// Cancels, pauses or resumes a run. `expected_version`, when given, must equal the run's
    /// current version (optimistic concurrency).
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `run:operate`; [`AppError::NotFound`];
    /// [`AppError::StateConflict`] when the run's state does not allow it;
    /// [`AppError::RevisionConflict`] when `expected_version` is stale;
    /// [`AppError::IdempotencyKeyReused`].
    #[allow(clippy::too_many_arguments)]
    pub async fn operate(
        &self,
        principal: &Principal,
        id: RunId,
        operation: RunOperation,
        reason: &str,
        expected_version: Option<u64>,
        key: &str,
        ctx: &RequestContext,
    ) -> Result<RunRecord, AppError> {
        principal.authorize(Permission::RunOperate)?;
        let idempotency = Idempotency {
            scope: format!("run.operate:{id}"),
            key: key.to_owned(),
            fingerprint: fingerprint(&[
                operation.name().as_bytes(),
                reason.as_bytes(),
                &expected_version.unwrap_or(u64::MAX).to_be_bytes(),
            ]),
        };
        let Some(mut tx) = self
            .store
            .begin(principal.tenant_id(), id, Lock::Wait)
            .await?
        else {
            return Err(self.missing(principal, id, ctx).await);
        };
        if tx.replay(&idempotency).await?.is_some() {
            drop(tx);
            return self.get(principal, id, ctx).await;
        }
        let version = tx.parts().1.version();
        if expected_version.is_some_and(|expected| expected != version) {
            return Err(AppError::RevisionConflict { current: version });
        }
        let now = self.clock.now();
        let mut effects = Effects::by(
            Actor::Principal(principal.subject().as_str().to_owned()),
            ctx.trace_id.clone(),
        );
        effects.audit = Some(
            AuditRecord::by(
                principal,
                operation.action(),
                ResourceType::Run,
                id.to_string(),
                ctx.trace_id.clone(),
                now,
            )
            .with_reason(reason.chars().take(512).collect::<String>()),
        );
        effects.idempotency = Some((idempotency, operation.name().to_owned()));
        self.engine
            .run(&mut tx, effects, |def, state, now| {
                let progress = match operation {
                    RunOperation::Cancel => {
                        execution::stop_run(def, state, StopReason::Cancelled, None, now)?
                    }
                    RunOperation::Pause => Progress {
                        events: vec![execution::pause_run(state)?],
                        pending: None,
                    },
                    RunOperation::Resume => execution::resume_run(def, state, now)?,
                };
                Ok(((), progress))
            })
            .await
            .map_err(engine_error)?;
        tx.commit().await?;
        self.get(principal, id, ctx).await
    }

    /// Delivers a signal to the run's waiting `waitEvent` steps whose event type is `name`. The
    /// payload becomes the output of each such step. Returns the nodes that were woken.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `run:signal`; [`AppError::NotFound`];
    /// [`AppError::StateConflict`] when the run is not waiting for that signal (a signal is never
    /// buffered or dropped silently); [`AppError::NotAllowed`] for a bad payload;
    /// [`AppError::IdempotencyKeyReused`].
    pub async fn signal(
        &self,
        principal: &Principal,
        id: RunId,
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
        let idempotency = Idempotency {
            scope: format!("run.signal:{id}"),
            key: key.to_owned(),
            fingerprint: fingerprint(&[name.as_bytes(), payload]),
        };
        let Some(mut tx) = self
            .store
            .begin(principal.tenant_id(), id, Lock::Wait)
            .await?
        else {
            return Err(self.missing(principal, id, ctx).await);
        };
        if let Some(delivered) = tx.replay(&idempotency).await? {
            return Ok(delivered
                .split(',')
                .filter(|n| !n.is_empty())
                .filter_map(|n| NodeId::parse(n).ok())
                .collect());
        }
        let targets: Vec<NodeId> = {
            let (def, state) = tx.parts();
            state
                .steps()
                .filter(|(_, step)| {
                    step.status() == StepStatus::Waiting
                        && matches!(step.waiting_on(), Some(WaitingOn::Event { .. }))
                })
                .filter(|(node, _)| {
                    matches!(
                        def.node(node).map(sagat_domain::workflow::Node::kind),
                        Some(NodeKind::WaitEvent { event_type, .. }) if event_type.as_str() == name
                    )
                })
                .map(|(node, _)| node.clone())
                .collect()
        };
        if targets.is_empty() {
            return Err(AppError::StateConflict(
                "the run is not waiting for that signal",
            ));
        }
        // What a run retains is bounded (every lease carries it): a signal that would exceed it is
        // refused instead of stored.
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
                id.to_string(),
                ctx.trace_id.clone(),
                now,
            )
            .with_reason(format!(
                "signal '{}'",
                name.chars().take(64).collect::<String>()
            )),
        );
        effects.outputs = targets
            .iter()
            .map(|n| (n.clone(), payload.to_vec()))
            .collect();
        effects.idempotency = Some((
            idempotency,
            targets
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>()
                .join(","),
        ));
        let woken = targets.clone();
        self.engine
            .run(&mut tx, effects, move |def, state, now| {
                let mut events = Vec::new();
                let mut pending = None;
                for node in &targets {
                    let progress = execution::satisfy_wait(def, state, node, now)?;
                    events.extend(progress.events);
                    pending = pending.or(progress.pending);
                }
                Ok(((), Progress { events, pending }))
            })
            .await
            .map_err(engine_error)?;
        tx.commit().await?;
        Ok(woken)
    }

    async fn namespace(&self, principal: &Principal, slug: &str) -> Result<Namespace, AppError> {
        let slug = sagat_domain::identity::Slug::parse(slug).map_err(|_| AppError::NotFound)?;
        self.store
            .find_namespace_by_slug(principal.tenant_id(), &slug)
            .await?
            .ok_or(AppError::NotFound)
    }

    async fn missing(&self, principal: &Principal, id: RunId, ctx: &RequestContext) -> AppError {
        if let Ok(true) = self
            .store
            .run_exists_outside(principal.tenant_id(), id)
            .await
        {
            let record = AuditRecord::by(
                principal,
                AuditAction::CrossTenantAccessDenied,
                ResourceType::Run,
                id.to_string(),
                ctx.trace_id.clone(),
                self.clock.now(),
            )
            .with_reason("resource not accessible from this tenant");
            if let Err(error) = RunStore::record_denial(&self.store, &record).await {
                tracing::error!(%error, action = %record.action(), "failed to record audit entry");
            }
        }
        AppError::NotFound
    }
}

/// Translates an engine failure into what a client may be told.
fn engine_error(error: EngineError) -> AppError {
    match error {
        EngineError::Store(store) => store.into(),
        EngineError::Execution(execution) => execution_error(&execution),
    }
}

/// Maps a refused domain command. Stale callers and finished runs are state conflicts; anything
/// else is a defect, logged and reported as an internal error.
#[must_use]
pub fn execution_error(error: &ExecutionError) -> AppError {
    match error {
        ExecutionError::RunFinished(_) => AppError::StateConflict("the run has already finished"),
        ExecutionError::AlreadyPaused => AppError::StateConflict("the run is already paused"),
        ExecutionError::NotPaused => AppError::StateConflict("the run is not paused"),
        ExecutionError::AlreadyStopping => {
            AppError::StateConflict("the run is already being stopped")
        }
        ExecutionError::RunNotRunnable(_) => {
            AppError::StateConflict("the run does not accept new work")
        }
        ExecutionError::StaleAttempt { .. } | ExecutionError::IllegalTransition { .. } => {
            AppError::StateConflict("the step is not in a state that allows this")
        }
        other => {
            tracing::error!(error = %other, "the execution engine refused a command it should accept");
            AppError::Internal
        }
    }
}

/// Length-prefixed concatenation, so `("ab", "c")` and `("a", "bc")` differ.
fn fingerprint(parts: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for part in parts {
        out.extend_from_slice(&(part.len() as u64).to_be_bytes());
        out.extend_from_slice(part);
    }
    out
}

/// The `RunEvent` list of a progress, for tests and diagnostics.
#[must_use]
pub fn event_names(events: &[RunEvent]) -> Vec<&'static str> {
    events.iter().map(RunEvent::name).collect()
}
