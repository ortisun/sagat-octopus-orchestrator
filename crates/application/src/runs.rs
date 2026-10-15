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
    self, ExecutionError, FailureCode, Progress, ReviewOutcome, RunEvent, RunId, RunStatus,
    StepStatus, StopReason, WaitingOn,
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
    pub input_blob: Option<crate::blobs::RunInputBinding>,
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
    /// Why it happened, when it is the result of something going wrong, a person's choice or the
    /// clock (FR-008-002).
    pub cause: Option<sagat_domain::execution::Cause>,
    /// The span of the attempt the event belongs to (16 hex), for the events of an attempt: the span
    /// the worker was given, which a trace backend shows.
    pub span_id: Option<String>,
    /// Every step the event created or changed: the one it names and the ones it activated
    /// (`RunEvent::changed_nodes`). The journal view shows `node` only; the changes of the graph
    /// need all of them. `None` for an event this build cannot decode, whose effect is not known.
    pub touched: Option<Vec<String>>,
}

/// One step as the runtime graph shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepView {
    /// Full logical identity of the projected execution; `node` remains the local definition name.
    pub instance: sagat_domain::execution::ExecutionInstanceId,
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
    /// The recorded case or default for a switch step.
    pub switch_selection: Option<sagat_domain::execution::SwitchSelection>,
}

/// What a run knows about one of its sagas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SagaView {
    pub node: NodeId,
    /// `FORWARD`, `ABORTING`, `COMPENSATING` or `DONE`.
    pub phase: String,
    /// Why the saga compensates, or did.
    pub cause: Option<String>,
    /// `COMPENSATED`, `PARTIAL` or `COMPENSATION_FAILED`, once the saga is done.
    pub outcome: Option<String>,
    /// Completed steps whose effect could not be undone.
    pub irreversible: Vec<NodeId>,
}

/// A run, its definition and its steps, read at one instant.
#[derive(Debug, Clone)]
pub struct GraphSnapshot {
    pub run: RunRecord,
    pub definition: Arc<WorkflowDefinition>,
    pub steps: Vec<StepView>,
    pub sagas: Vec<SagaView>,
}

/// Most journal events a [`GraphChanges`] carries; a client further behind than this is given the
/// whole graph, which is cheaper than replaying a long history node by node.
pub const MAX_CHANGE_EVENTS: u32 = 500;

/// What happened to a run's graph after a journal sequence (the live view, D141): the graph as it
/// is now, and the journal events that led from the client's sequence to it, so the client can
/// update the nodes those events touched and keep the rest.
#[derive(Debug, Clone)]
pub struct GraphChanges {
    pub graph: GraphSnapshot,
    /// The sequence the client had.
    pub since: u64,
    /// The client must replace what it has with the whole graph.
    pub reset: bool,
    /// Events with `since < seq <= graph.run.version`, ascending; empty on a reset.
    pub events: Vec<JournalEntry>,
}

impl GraphChanges {
    /// The sequence the graph is at.
    #[must_use]
    pub fn projection_seq(&self) -> u64 {
        self.graph.run.version
    }
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
pub trait RunStore: QuotaLookup + crate::telemetry::TelemetryStore + Send + Sync {
    type Tx: RunTx;

    /// Reads an already committed start before resolving an ephemeral upload. `create` repeats
    /// this decision transactionally; conflicting fingerprints are never treated as misses.
    fn replay_start(
        &self,
        tenant: TenantId,
        idempotency: &Idempotency,
    ) -> impl Future<Output = Result<Option<String>, StoreError>> + Send;

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

    /// What a run is started again from: the namespace it ran in, the input it was given. `None`
    /// when the run does not exist.
    fn rerun_source(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> impl Future<Output = Result<Option<RerunSource>, StoreError>> + Send;

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

/// What an operator may do to a step that waits for a decision (`FR-003-005`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewAction {
    /// Another attempt of the step; the old attempts stay in the journal. For a step that waits
    /// out the delay of a retry, the attempt starts now.
    Retry,
    /// Hold a step that has not started (ready, or waiting out a retry delay) for a person to
    /// decide: nothing more happens to it until someone retries it or settles it.
    Hold,
    /// Give up on a compensation. Needs explicit permission; the saga then ends `PARTIAL`.
    Skip,
    /// Settle the step: it happened (with the output it would have had), or it did not.
    Resolve(ReviewOutcome),
}

impl ReviewAction {
    const fn audit(&self) -> AuditAction {
        match self {
            Self::Retry => AuditAction::SagaStepRetried,
            Self::Hold => AuditAction::RunMarkedForReview,
            Self::Skip => AuditAction::SagaStepSkipped,
            Self::Resolve(_) => AuditAction::SagaStepResolved,
        }
    }

    fn name(&self) -> String {
        match self {
            Self::Retry => "retry".to_owned(),
            Self::Hold => "hold".to_owned(),
            Self::Skip => "skip".to_owned(),
            Self::Resolve(ReviewOutcome::Succeeded) => "resolve:succeeded".to_owned(),
            Self::Resolve(ReviewOutcome::Failed(code)) => format!("resolve:failed:{code}"),
        }
    }
}

/// The shortest and longest justification of an operator's decision, in characters, after trimming.
pub const MIN_JUSTIFICATION: usize = 8;
pub const MAX_JUSTIFICATION: usize = 2000;

fn outcome_label(outcome: &ReviewOutcome) -> String {
    match outcome {
        ReviewOutcome::Succeeded => "SUCCEEDED".to_owned(),
        ReviewOutcome::Failed(code) => format!("FAILED({code})"),
    }
}

/// What a run is started again from (`RERUN_AS_NEW`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RerunSource {
    /// The slug of the namespace the run ran in.
    pub namespace: String,
    /// The input the run was given.
    pub input: Vec<u8>,
}

/// Why and from what a run is started again.
#[derive(Debug, Clone, Copy)]
pub struct Rerun<'a> {
    pub of: RunId,
    pub reason: &'a str,
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
    lag_budget: Duration,
    input_blobs: Option<(
        crate::blobs::BlobService,
        Arc<dyn crate::blobs::RunInputChecker>,
    )>,
}

/// The state of a run's telemetry and what it is judged by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunTelemetryView {
    pub state: crate::telemetry::TelemetryState,
    pub telemetry: crate::telemetry::RunTelemetry,
    /// How late spans may be before the state is `BEHIND`.
    pub budget: Duration,
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
            lag_budget: Duration::from_secs(30),
            input_blobs: None,
        }
    }

    /// How late a run's spans may be before its telemetry is called behind (default 30 s).
    #[must_use]
    pub const fn with_lag_budget(mut self, budget: Duration) -> Self {
        self.lag_budget = budget;
        self
    }

    #[must_use]
    pub fn with_input_blobs(
        mut self,
        blobs: crate::blobs::BlobService,
        checker: Arc<dyn crate::blobs::RunInputChecker>,
    ) -> Self {
        self.input_blobs = Some((blobs, checker));
        self
    }

    #[must_use]
    pub fn input_blobs(&self) -> Option<&crate::blobs::BlobService> {
        self.input_blobs.as_ref().map(|(service, _)| service)
    }

    /// Authorizes the namespace of a pre-run upload before its stream is read.
    pub async fn input_blob_access(
        &self,
        principal: &Principal,
        namespace: &str,
    ) -> Result<crate::blobs::BlobAccess, AppError> {
        principal.authorize(Permission::RunStart)?;
        let namespace = self.namespace(principal, namespace).await?;
        Ok(crate::blobs::BlobAccess {
            tenant_id: principal.tenant_id(),
            namespace_id: namespace.id(),
        })
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
        self.start_with(principal, request, None, None, ctx).await
    }

    /// Starts with a staged upload belonging to this principal. The external bytes stay opaque;
    /// the canonical descriptor is the input checked by the workflow's schema.
    pub async fn start_with_blob(
        &self,
        principal: &Principal,
        request: &StartRun<'_>,
        id: sagat_domain::blobs::BlobId,
        ctx: &RequestContext,
    ) -> Result<RunRecord, AppError> {
        self.start_with(principal, request, None, Some(id), ctx)
            .await
    }

    /// Starts a run again from an ended one (`RERUN_AS_NEW`): the same version of the same workflow,
    /// the same input and correlation id, a run of its own that says where it came from. An
    /// operator's justification goes to the audit trail.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `run:operate` and `run:start`; [`AppError::NotAllowed`]
    /// without a justification; [`AppError::NotFound`] for a run the caller cannot see;
    /// [`AppError::StateConflict`] when the run has not ended (starting it again would do its work
    /// twice); the errors of [`Self::start`].
    pub async fn rerun(
        &self,
        principal: &Principal,
        id: RunId,
        reason: &str,
        key: &str,
        ctx: &RequestContext,
    ) -> Result<RunRecord, AppError> {
        principal.authorize(Permission::RunOperate)?;
        principal.authorize(Permission::RunStart)?;
        let reason = reason.trim();
        let length = reason.chars().count();
        if !(MIN_JUSTIFICATION..=MAX_JUSTIFICATION).contains(&length)
            || reason.chars().any(char::is_control)
        {
            return Err(AppError::NotAllowed(
                "a justification of 8 to 2000 characters, in plain text, is required",
            ));
        }
        let original = self.get(principal, id, ctx).await?;
        if !original.status.is_terminal() {
            return Err(AppError::StateConflict(
                "only a run that has ended can be started again",
            ));
        }
        let Some(source) = self.store.rerun_source(principal.tenant_id(), id).await? else {
            return Err(self.missing(principal, id, ctx).await);
        };
        self.start_with(
            principal,
            &StartRun {
                namespace: &source.namespace,
                workflow: original.info.workflow_id,
                version: Some(original.info.version_id),
                input: &source.input,
                correlation_id: original.info.correlation_id.as_deref(),
                deadline: None,
                key,
            },
            Some(Rerun { of: id, reason }),
            None,
            ctx,
        )
        .await
    }

    async fn start_with(
        &self,
        principal: &Principal,
        request: &StartRun<'_>,
        rerun: Option<Rerun<'_>>,
        input_blob: Option<sagat_domain::blobs::BlobId>,
        ctx: &RequestContext,
    ) -> Result<RunRecord, AppError> {
        principal.authorize(Permission::RunStart)?;
        if input_blob.is_none() && request.input.len() > MAX_PAYLOAD_BYTES {
            return Err(AppError::NotAllowed("the run input is larger than 256 KiB"));
        }
        if input_blob.is_none() && !self.engine.evaluator().is_json_object(request.input) {
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
        let blob_idempotency = input_blob.map(|id| Idempotency {
            scope: format!("run.start:{}", namespace.id()),
            key: request.key.to_owned(),
            fingerprint: fingerprint(&[
                b"run.input.blob",
                principal.subject().as_str().as_bytes(),
                workflow.id().to_string().as_bytes(),
                request
                    .version
                    .map(|v| v.to_string())
                    .unwrap_or_default()
                    .as_bytes(),
                id.to_string().as_bytes(),
                request.correlation_id.unwrap_or_default().as_bytes(),
                request
                    .deadline
                    .map(|d| d.unix_timestamp_nanos().to_be_bytes())
                    .unwrap_or_default()
                    .as_slice(),
            ]),
        });
        if let Some(idempotency) = &blob_idempotency {
            // A stable request identity does not depend on the catalog still containing an unused
            // upload, or on which version is active when a lost response is retried.
            if let Some(existing) = self
                .store
                .replay_start(principal.tenant_id(), idempotency)
                .await?
            {
                return self.replayed_start(principal, &existing).await;
            }
        }
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
            parent_span: ctx.span_id.clone(),
            rerun_of: rerun.map(|r| r.of),
            deadline_at: request.deadline,
            created_at: now,
            parent: None,
            depth: 0,
            root: run_id,
        };
        let audit = match rerun {
            None => AuditRecord::by(
                principal,
                AuditAction::RunStarted,
                ResourceType::Run,
                run_id.to_string(),
                ctx.trace_id.clone(),
                now,
            ),
            Some(rerun) => AuditRecord::by(
                principal,
                AuditAction::RunRerun,
                ResourceType::Run,
                run_id.to_string(),
                ctx.trace_id.clone(),
                now,
            )
            .with_reason(format!("started again from {}: {}", rerun.of, rerun.reason)),
        };
        let mut idempotency = Idempotency {
            scope: format!("run.start:{}", namespace.id()),
            key: request.key.to_owned(),
            fingerprint: fingerprint(&[
                rerun
                    .map(|r| r.of.to_string())
                    .unwrap_or_default()
                    .as_bytes(),
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
        if let Some(blob_idempotency) = blob_idempotency {
            idempotency = blob_idempotency;
        }
        let (input, input_binding) = match input_blob {
            Some(id) => {
                let (blobs, checker) = self.input_blobs.as_ref().ok_or(AppError::Unavailable)?;
                let access = crate::blobs::BlobAccess {
                    tenant_id: principal.tenant_id(),
                    namespace_id: namespace.id(),
                };
                let descriptor = match blobs
                    .describe_input(access, id, principal.subject().as_str())
                    .await
                {
                    Ok(descriptor) => descriptor,
                    Err(crate::blobs::BlobReadError::NotFound) => {
                        // A peer may have committed this exact start after the first replay read.
                        if let Some(existing) = self
                            .store
                            .replay_start(principal.tenant_id(), &idempotency)
                            .await?
                        {
                            return self.replayed_start(principal, &existing).await;
                        }
                        return Err(AppError::NotFound);
                    }
                    Err(_) => return Err(AppError::Unavailable),
                };
                let input = crate::blobs::descriptor_json(&descriptor);
                checker
                    .check(&version.document, &input)
                    .map_err(AppError::NotAllowed)?;
                (
                    input,
                    Some(crate::blobs::RunInputBinding::Uploaded(descriptor)),
                )
            }
            None => (
                request.input.to_vec(),
                rerun.map(|r| crate::blobs::RunInputBinding::Inherit(r.of)),
            ),
        };
        let limits = RunLimits::of(&self.store, principal.tenant_id(), namespace.id()).await?;
        let new = NewRun {
            limits,
            info,
            definition: Arc::clone(&definition),
            input,
            input_blob: input_binding,
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

    async fn replayed_start(
        &self,
        principal: &Principal,
        existing: &str,
    ) -> Result<RunRecord, AppError> {
        let id = existing.parse::<RunId>().map_err(|_| AppError::Internal)?;
        self.store
            .get(principal.tenant_id(), id)
            .await?
            .ok_or(AppError::Internal)
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

    /// How far the run's spans and measurements have got in relation to its journal (the timeline's
    /// projection state). The journal and the graph are as current as the last commit; this is
    /// about what is derived from them.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `run:read`; [`AppError::NotFound`].
    pub async fn telemetry(
        &self,
        principal: &Principal,
        id: RunId,
        ctx: &RequestContext,
    ) -> Result<RunTelemetryView, AppError> {
        principal.authorize(Permission::RunRead)?;
        let Some(telemetry) = self.store.run_telemetry(principal.tenant_id(), id).await? else {
            return Err(self.missing(principal, id, ctx).await);
        };
        Ok(RunTelemetryView {
            state: telemetry.state(self.lag_budget),
            telemetry,
            budget: self.lag_budget,
        })
    }

    /// The attempts of the run's steps as the journal tells them (FR-008-002): when each was
    /// leased, how it ended and why, the span a trace backend shows it as. A journal longer than
    /// the store gives in one batch is read in as many as it takes.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `run:read`; [`AppError::NotFound`].
    pub async fn attempts(
        &self,
        principal: &Principal,
        id: RunId,
        ctx: &RequestContext,
    ) -> Result<Vec<crate::telemetry::Attempt>, AppError> {
        principal.authorize(Permission::RunRead)?;
        match crate::telemetry::all_attempts(
            &self.store,
            principal.tenant_id(),
            id,
            self.clock.now(),
        )
        .await?
        {
            Some(attempts) => Ok(attempts),
            None => {
                // No trace to derive from (or no run): a run the caller may see has no attempts to
                // show, a run they may not is a 404.
                if self
                    .store
                    .run_telemetry(principal.tenant_id(), id)
                    .await?
                    .is_some()
                {
                    Ok(Vec::new())
                } else {
                    Err(self.missing(principal, id, ctx).await)
                }
            }
        }
    }

    /// What changed in the run's graph after journal sequence `since`.
    ///
    /// Every transition of a run is a journal event with a sequence, so a client that reads the
    /// changes after the last sequence it saw misses nothing, however many snapshots it skipped.
    /// A `since` ahead of the run, or more than [`MAX_CHANGE_EVENTS`] events behind it, is a
    /// `reset`: the answer is the whole graph.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `run:read`; [`AppError::NotFound`].
    pub async fn graph_changes(
        &self,
        principal: &Principal,
        id: RunId,
        since: u64,
        ctx: &RequestContext,
    ) -> Result<GraphChanges, AppError> {
        let graph = self.graph(principal, id, ctx).await?;
        let projection = graph.run.version;
        if since > projection {
            return Ok(GraphChanges {
                graph,
                since,
                reset: true,
                events: Vec::new(),
            });
        }
        if since == projection {
            return Ok(GraphChanges {
                graph,
                since,
                reset: false,
                events: Vec::new(),
            });
        }
        let Some(mut events) = self
            .store
            .journal(
                principal.tenant_id(),
                id,
                Some(since),
                MAX_CHANGE_EVENTS + 1,
            )
            .await?
        else {
            return Err(self.missing(principal, id, ctx).await);
        };
        // What the snapshot already includes and no more: events after it are the next read's.
        events.retain(|event| event.seq <= projection);
        // More events than a delta carries, or one whose effect on the graph is not known (an event of a
        // newer engine): the whole graph is the answer that cannot be wrong.
        let reset = events.len() > MAX_CHANGE_EVENTS as usize
            || events.iter().any(|event| event.touched.is_none());
        if reset {
            events.clear();
        }
        Ok(GraphChanges {
            graph,
            since,
            reset,
            events,
        })
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

    /// An operator decides what happens to a step that waits for one: a compensation that failed
    /// after its retries, or a step whose outcome nobody could establish.
    ///
    /// Retrying needs `run:operate`; skipping a compensation and settling a step by hand need
    /// `saga:override`. All three need a justification, which is kept in the audit trail together
    /// with who asked; the journal keeps what the step did before and after, and nothing is erased.
    ///
    /// # Errors
    /// [`AppError::Forbidden`]; [`AppError::NotAllowed`] without a justification, or to skip
    /// something that is not a compensation; [`AppError::NotFound`] for a run or a step the caller
    /// cannot see; [`AppError::StateConflict`] when the step does not wait for an operator (a second
    /// command on the same step: the first one won); [`AppError::IdempotencyKeyReused`].
    #[allow(clippy::too_many_arguments)]
    pub async fn review(
        &self,
        principal: &Principal,
        id: RunId,
        node: &NodeId,
        action: &ReviewAction,
        reason: &str,
        expected_version: Option<u64>,
        key: &str,
        ctx: &RequestContext,
    ) -> Result<RunRecord, AppError> {
        principal.authorize(match action {
            ReviewAction::Retry | ReviewAction::Hold => Permission::RunOperate,
            ReviewAction::Skip | ReviewAction::Resolve(_) => Permission::SagaOverride,
        })?;
        // What is audited is what the person wrote, bounded the same way whoever asks: the API
        // counts characters after trimming, and so does this, so a justification of one letter and
        // seven spaces is not one.
        let reason = reason.trim();
        let length = reason.chars().count();
        if !(MIN_JUSTIFICATION..=MAX_JUSTIFICATION).contains(&length)
            || reason.chars().any(char::is_control)
        {
            return Err(AppError::NotAllowed(
                "a justification of 8 to 2000 characters, in plain text, is required",
            ));
        }
        if *action == ReviewAction::Skip && !node.is_compensation() {
            return Err(AppError::NotAllowed("only a compensation can be skipped"));
        }
        let idempotency = Idempotency {
            scope: format!("run.review:{id}"),
            key: key.to_owned(),
            fingerprint: fingerprint(&[
                node.as_str().as_bytes(),
                action.name().as_bytes(),
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
        if tx.parts().1.step(node).is_none() {
            return Err(AppError::NotFound);
        }
        let version = tx.parts().1.version();
        if expected_version.is_some_and(|expected| expected != version) {
            return Err(AppError::RevisionConflict { current: version });
        }
        // Asking again for an effect that may already have happened is the decision the platform
        // would not take by itself (a high risk step waits for a person for that reason), so it is
        // an administrator's to take: an operator may retry a step that is known not to have
        // happened, or a compensation (which is idempotent by contract).
        // A step that is only waiting out the delay of a retry is not that decision: the platform
        // had already decided to try again, and the operator is not waiting for the delay.
        if *action == ReviewAction::Retry
            && !node.is_compensation()
            && tx
                .parts()
                .1
                .step(node)
                .is_some_and(|step| step.status() == StepStatus::Waiting)
            && tx
                .parts()
                .1
                .step(node)
                .and_then(|step| step.failure())
                .is_some_and(FailureCode::leaves_outcome_unknown)
        {
            principal.authorize(Permission::SagaOverride)?;
        }
        let now = self.clock.now();
        let mut effects = Effects::by(
            Actor::Principal(principal.subject().as_str().to_owned()),
            ctx.trace_id.clone(),
        );
        // The audit record says what was decided, not only that something was: a step settled by
        // hand is recorded with the outcome that was declared.
        let audited = match action {
            ReviewAction::Resolve(outcome) => {
                format!("declared {}: {reason}", outcome_label(outcome))
            }
            ReviewAction::Retry | ReviewAction::Hold | ReviewAction::Skip => reason.to_owned(),
        };
        effects.audit = Some(
            AuditRecord::by(
                principal,
                action.audit(),
                ResourceType::Run,
                format!("{id}/{node}"),
                ctx.trace_id.clone(),
                now,
            )
            .with_reason(audited),
        );
        effects.idempotency = Some((idempotency, action.name()));
        let step = node.clone();
        let action = action.clone();
        self.engine
            .run(&mut tx, effects, move |def, state, now| {
                let progress = match &action {
                    ReviewAction::Retry => execution::retry_review(def, state, &step, now)?,
                    ReviewAction::Hold => execution::mark_for_review(def, state, &step, now)?,
                    ReviewAction::Skip => execution::skip_review(def, state, &step, now)?,
                    ReviewAction::Resolve(outcome) => {
                        execution::resolve_review(def, state, &step, outcome.clone(), now)?
                    }
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
