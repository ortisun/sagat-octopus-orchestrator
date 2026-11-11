//! Run use cases: start, read, operate and signal (`FR-002-001`..`FR-002-005`, `FR-002-007`).
//!
//! Every use case receives an authenticated [`Principal`], authorizes before touching storage,
//! derives the tenant from it, and performs each state change through [`Engine::run`] inside a
//! [`RunTx`], so the journal, the projections, the audit record and the idempotency record commit
//! together or not at all.

mod ad_hoc;
mod batch_items;
mod pending_reset;
mod reapply;
mod reset;
pub(crate) use ad_hoc::audit_denied as audit_database_execution_denied;
pub use ad_hoc::{
    AdHocMode, ConnectionProbe, RetainedDefinition, RunOrigin, StartDatabaseExecution,
};
pub use pending_reset::{
    PendingAdmission, PendingRegistration, PendingReset, RESET_ALREADY_COMPLETED, ResetSourcePolicy,
};
pub use reapply::{
    Candidate, Delivery, Material, PlannedReapply, REAPPLY_CEILING, RESET_KEY_PREFIX, ReapplyEntry,
    ReapplyKind, ReapplyOutcome, ReapplyReason, ReportEntry, TooMany, candidates, count_reapply,
    end_signals_at_first_skip, reapply_key, report_from_json, report_json, update_answer,
    update_refusal,
};
pub use reset::{
    RESET_PREFIX_EVENTS, ResetMade, ResetPointInfo, ResetPointKind, ResetPreview, ResetRecord,
    ResetRegistration, ResetRequest, ResetSource, ResetState, RestoredPrefix, SourceEvent,
    prefix_digest,
};

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::business_ids::{BusinessId, ConflictPolicy, ReusePolicy, ReuseWindow};
use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::execution::{
    self, ExecutionError, ExecutionInstanceId, OperationAddress, OperationScope, Progress,
    ReviewOutcome, RunEvent, RunId, RunStatus, StepStatus, StopReason, UpdateKey, UpdateRequest,
    WaitVia, WaitingOn,
};
use sagat_domain::identity::{Namespace, NamespaceId, Permission, Principal, TenantId};
use sagat_domain::workflow::{ActivityName, Digest, NodeId, NodeKind, WorkflowDefinition};
use time::OffsetDateTime;

use crate::clock::Clock;
use crate::declared_operations::{AppliedInTx, DeclaredOperationEvaluator, DeclaredUpdate};
use crate::definitions::{DefinitionStore, Idempotency};
use crate::error::{AppError, StoreError};
use crate::execution::{
    Actor, DefinitionCache, Effects, Engine, EngineError, ExpressionEvaluator, MAX_PAYLOAD_BYTES,
    RunInfo, RunRecord, RunTx,
};
use crate::identity::{Page, PageRequest, RequestContext};
use crate::operation_evaluation::DeclaredOperationArguments;
use crate::quotas::{QuotaLookup, RunLimits};

/// Longest a run may be given to finish.
pub const MAX_RUN_DEADLINE: Duration = Duration::from_secs(366 * 24 * 60 * 60);
/// Longest correlation id accepted at start.
pub const MAX_CORRELATION_ID: usize = 160;

/// Why a run cannot start from a workflow none of whose versions is published
/// ([`AppError::StateConflict`]). A schedule keeps such a time owed and tries it again.
pub const NO_PUBLISHED_VERSION: &str = "the workflow has no published version";

/// How a run is opened for a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lock {
    /// Wait for the run's row lock (API commands, timers).
    Wait,
    /// Give up at once when another transaction holds it (workers polling for tasks).
    SkipLocked,
}

/// The business identifier a start carries and what to do when it is held (feature 018,
/// `FR-018-018..020`). Its `Debug` hides the identifier: it is customer text.
#[derive(Debug, Clone)]
pub struct BusinessStart {
    pub id: BusinessId,
    pub conflict: ConflictPolicy,
    pub reuse: ReusePolicy,
    /// The window the binding is frozen with when it is made.
    pub window: ReuseWindow,
}

/// The longest start delay (feature 018, `FR-018-023`): 366 days.
pub const MAX_START_DELAY: Duration = Duration::from_secs(366 * 86_400);

/// What a start can carry beyond its request (feature 018). `Default` is a start with none.
#[derive(Debug, Clone, Default)]
pub struct StartOptions {
    pub business: Option<BusinessStart>,
    /// The run is created `WAITING` and releases its entry once, this long after it was created
    /// (1 ms to 366 days). It counts toward the quotas and holds its business identifier meanwhile.
    pub start_delay: Option<Duration>,
}

/// How a business identifier stands (feature 018, `FR-018-022`): who holds it, whether that
/// execution is still open, how it ended, and until when a closed holder is remembered. Its `Debug`
/// hides the identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusinessBinding {
    pub business_id: BusinessId,
    /// The execution that holds the identifier: the run, or the initial run of a continuation chain.
    pub execution_id: RunId,
    /// The run that carries the work now: the run itself, or the current segment of the chain.
    pub current_run_id: RunId,
    /// The holder has not reached a final outcome (a chain is open until it has one).
    pub open: bool,
    /// How the holder ended, when it has.
    pub last_status: Option<RunStatus>,
    pub workflow_id: WorkflowId,
    pub bound_at: OffsetDateTime,
    /// When `REJECT_DUPLICATE` stops applying to a closed holder.
    pub reusable_after: Option<OffsetDateTime>,
}

/// Content of the tenant that a reset reads to deliver it again: how long it is, and the bytes unless they were not asked
/// for. Its `Debug` never prints them.
#[derive(Clone, PartialEq, Eq)]
pub struct Retained {
    pub size: u64,
    pub content: Option<Vec<u8>>,
}

impl std::fmt::Debug for Retained {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Retained")
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

/// What a start came to: the run, and whether it already existed (an open execution held the
/// business identifier and the start asked for `USE_EXISTING`: nothing was created).
#[derive(Debug, Clone)]
pub struct Started {
    pub run: RunRecord,
    pub existing: bool,
    /// The steps a signal delivered with the start woke (Signal-with-Start); empty otherwise.
    pub delivered_to: Vec<NodeId>,
    /// The declared Update applied with the start (Update-with-Start), or its stable rejection.
    pub update: Option<AppliedInTx>,
}

/// The signal a Signal-with-Start request delivers (feature 018, `FR-018-024`).
#[derive(Debug, Clone, Copy)]
pub struct SignalAtStart<'a> {
    pub name: &'a str,
    pub payload: &'a [u8],
}

/// What a Signal-with-Start came to: the run, whether this request created it, and the steps woken.
#[derive(Debug, Clone)]
pub struct SignalStarted {
    pub run: RunRecord,
    pub started: bool,
    pub delivered_to: Vec<NodeId>,
}

/// The declared Update an Update-with-Start request applies (feature 018, `FR-018-025`).
#[derive(Debug, Clone, Copy)]
pub struct UpdateAtStart<'a> {
    pub operation: &'a ActivityName,
    pub update_key: &'a UpdateKey,
    pub arguments: &'a DeclaredOperationArguments,
    pub expected_state_revision: Option<u64>,
    /// The batch job that applies it, recorded in the audit of the change.
    pub batch_job: Option<uuid::Uuid>,
}

/// What an Update-with-Start came to: the run, whether this request created it, and the Update's
/// resolution (applied with its result, or a business rejection the run was started with).
#[derive(Debug, Clone)]
pub struct UpdateStarted {
    pub run: RunRecord,
    pub started: bool,
    pub update: AppliedInTx,
}

/// What a start carries along besides its request.
#[derive(Debug, Clone, Copy)]
enum WithStart<'a> {
    Signal(SignalAtStart<'a>),
    Update(UpdateAtStart<'a>),
}

/// What a signal to a run that waits for nothing is refused with.
const NOT_WAITING_FOR_SIGNAL: &str = "the run is not waiting for that signal";

/// What becomes of the receipt of a start that an existing holder served (feature 018, `FR-018-026`).
/// The holder is not created by the request, so `create` leaves no receipt of its own; without one a
/// retry made after the holder closed would claim the free identifier and take effect a second time.
#[derive(Clone, Copy)]
enum HolderReceipt<'a> {
    /// Nothing to keep or to read: a batch item, whose key is the item's own.
    None,
    /// The holder serves this request now: keep its receipts with the effect.
    Keep(&'a Idempotency),
    /// A retry: say whether the first request was served by a holder.
    Read(&'a Idempotency),
}

/// The record that says a holder, not a new run, answered the request under `start`.
fn served_marker(start: &Idempotency) -> Idempotency {
    Idempotency {
        scope: format!("{}.served", start.scope),
        key: start.key.clone(),
        fingerprint: start.fingerprint.clone(),
    }
}

/// What creating a run came to.
#[derive(Debug)]
pub enum Admitted<T> {
    /// The run was created: here is its open transaction.
    Created(T),
    /// An earlier call with the same key and request created it; carries its id.
    Replayed(String),
    /// The business identifier is held by an open execution and the start asked for
    /// `USE_EXISTING`: nothing was created; carries the holder's id.
    Existing(String),
}

/// A run about to be created.
#[derive(Debug, Clone)]
pub struct NewRun {
    /// Private definition to retain atomically with this run; only managed admission constructs it.
    pub retained: Option<RetainedDefinition>,
    /// The quotas the creation is judged against, atomically with the insert.
    pub limits: RunLimits,
    pub info: RunInfo,
    pub definition: Arc<WorkflowDefinition>,
    pub input: Vec<u8>,
    pub input_blob: Option<crate::blobs::RunInputBinding>,
    pub audit: AuditRecord,
    pub idempotency: Idempotency,
    /// The schedule action this run is the result of, committed with the run (feature 018).
    pub schedule: Option<crate::schedules::ScheduleAdmission>,
    /// The business identifier claimed in the creation transaction (feature 018).
    pub business: Option<BusinessStart>,
    /// Set when the run is created by a reset: the events `1..=N` of `info.rerun_of` are re-emitted under
    /// it, and this is `N` (feature 018, `FR-018-037`).
    pub reset_sequence: Option<u64>,
    /// Set when the run completes a reset that waited for its source (`CANCEL_SOURCE`, feature 018): the entry is
    /// locked, the lease of its shard and the authority of the requester are checked where the run is admitted.
    pub pending_reset: Option<PendingAdmission>,
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
    /// Every logical instance the event created or changed. The journal view still shows the local
    /// `node`; graph deltas need the complete scope path to distinguish repeated activations.
    /// `None` means this build could not decode the event and must return a full reset.
    pub touched: Option<Vec<sagat_domain::execution::ExecutionInstanceId>>,
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
    /// The approval request an approval step opened and how it stands; none before it opens.
    pub approval: Option<ApprovalView>,
}

/// How the approval request of an approval step stands. It names the outcome and the route that
/// followed, never who decided or why: the principal, the decision key and the reason code stay
/// in the approval's own audited endpoints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalView {
    pub status: sagat_domain::execution::ApprovalStatus,
    /// The pinned target the outcome routed to; none while pending or after cancellation.
    pub routed_node: Option<NodeId>,
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
    pub chain: Option<ChainView>,
}

/// The durable chain links visible for this run at the graph snapshot's revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainView {
    pub chain_id: RunId,
    pub segment_index: u32,
    pub segment_count: u32,
    pub current_run_id: RunId,
    pub predecessor_run_id: Option<RunId>,
    pub successor_run_id: Option<RunId>,
    pub completion_kind: Option<String>,
    pub final_outcome: Option<String>,
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
    pub origin: Option<RunOrigin>,
    /// The runs started under this business identifier, whole (feature 018, `FR-018-022`).
    pub business_id: Option<BusinessId>,
    /// The runs whose business identifier starts with this text. A literal: nothing in it is a
    /// wildcard. Visible ASCII, at most 200 characters, like an identifier.
    pub business_id_prefix: Option<String>,
    /// The runs this schedule started, whatever the kind of action (feature 018, `FR-018-022`).
    pub schedule: Option<sagat_domain::schedules::ScheduleId>,
}

/// Storage port for runs. Implemented by `sagat-infra-postgres`.
///
/// Every method takes the tenant explicitly; adapters MUST apply it as a filter on every statement.
pub trait RunStore:
    QuotaLookup + crate::telemetry::TelemetryStore + crate::authz::NamespaceDirectory + Send + Sync
{
    type Tx: RunTx;

    /// The run that carries the work of the execution `run` now: `run` itself, unless it is the
    /// initial run of a continuation chain whose current segment is another (feature 018: a signal or
    /// an Update meant for the execution that holds a business identifier reaches that segment).
    fn current_segment(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> impl Future<Output = Result<RunId, StoreError>> + Send;

    /// Reads an already committed start before resolving an ephemeral upload. `create` repeats
    /// this decision transactionally; conflicting fingerprints are never treated as misses.
    fn replay_start(
        &self,
        tenant: TenantId,
        subject: &str,
        idempotency: &Idempotency,
    ) -> impl Future<Output = Result<Option<String>, StoreError>> + Send;

    /// Creates the run (row, input payload, empty state) and returns its open transaction, or
    /// reports that the same request already created one.
    fn create(
        &self,
        new: NewRun,
    ) -> impl Future<Output = Result<Admitted<Self::Tx>, StoreError>> + Send;

    /// Private immutable document, only for its initiating actor's rerun.
    fn private_definition(
        &self,
        tenant: TenantId,
        actor: &str,
        run: RunId,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, StoreError>> + Send;

    fn latest_connection_probe(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        name: &sagat_domain::identity::Slug,
    ) -> impl Future<Output = Result<Option<ConnectionProbe>, StoreError>> + Send;

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

    /// How the business identifier stands in the namespace, `None` when it is unbound, when its
    /// holder is gone, or when it was closed for longer than its window (it is free then).
    fn business_binding(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        id: &BusinessId,
    ) -> impl Future<Output = Result<Option<BusinessBinding>, StoreError>> + Send;

    /// What a run is started again from: the namespace it ran in, the input it was given. `None`
    /// when the run does not exist.
    fn rerun_source(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> impl Future<Output = Result<Option<RerunSource>, StoreError>> + Send;

    /// The whole journal of a run as a reset reads it (feature 018, `FR-018-035`): every row that decodes, in
    /// order, with what a reset keeps of it, and the type of the first row that does not. `None` when the
    /// run does not exist.
    fn reset_source(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> impl Future<Output = Result<Option<ResetSource>, StoreError>> + Send;

    /// The resets made from `source`, newest first, at most 200 (feature 018, the register `run_resets`).
    fn run_resets(
        &self,
        tenant: TenantId,
        source: RunId,
    ) -> impl Future<Output = Result<Vec<ResetRecord>, StoreError>> + Send;

    /// One reset made from `source`; `None` when there is none with that id or another run made it.
    fn run_reset(
        &self,
        tenant: TenantId,
        source: RunId,
        reset: sagat_domain::execution::ResetId,
    ) -> impl Future<Output = Result<Option<ResetRecord>, StoreError>> + Send;

    /// The result stored for a reset request that waits for its source (the id of its entry): `None` when the key
    /// made none, [`StoreError::IdempotencyKeyReused`] when it made another one (feature 018, `CANCEL_SOURCE`).
    fn reset_receipt(
        &self,
        tenant: TenantId,
        idempotency: &Idempotency,
    ) -> impl Future<Output = Result<Option<String>, StoreError>> + Send;

    /// The reset that created `run`; `None` when no reset did.
    fn reset_made(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> impl Future<Output = Result<Option<ResetRecord>, StoreError>> + Send;

    /// What the tenant keeps under each digest, in the order asked, `None` for one it no longer keeps (the payload
    /// of a signal that a reset delivers again is the one its `wait.satisfied` row names; feature 018,
    /// `FR-018-039`). One statement for all of them. With `read` false the content is not read: the length says it is
    /// kept.
    fn payload_contents(
        &self,
        tenant: TenantId,
        digests: &[Vec<u8>],
        read: bool,
    ) -> impl Future<Output = Result<Vec<Option<Retained>>, StoreError>> + Send;

    /// The canonical arguments that the Updates recorded by the rows `sequences` of `run` kept for a reset
    /// (`reapplyOnReset`), in the order asked, `None` for the ones no longer kept. A row may have been restored into
    /// `run` by a reset: the arguments are on the receipt of the run that applied the Update first, which is found
    /// by following the resets that made `run` (G1978 (1)): the chain is read once for all the rows, at most 64
    /// resets deep, and a row beyond that is `None`. With `read` false the content is not read.
    fn retained_update_arguments(
        &self,
        tenant: TenantId,
        run: RunId,
        sequences: &[u64],
        read: bool,
    ) -> impl Future<Output = Result<Vec<Option<Retained>>, StoreError>> + Send;

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

/// The text that names a step in an audit record and in the fingerprint of a decision: its node for
/// a root step, and the iteration path before the node for a step of a scope
/// (`lines[1]/reserve~c`).
fn instance_label(instance: &ExecutionInstanceId) -> String {
    let mut label = String::new();
    for segment in instance.path().segments() {
        label.push_str(segment.scope_node().as_str());
        label.push('[');
        label.push_str(&segment.iteration_index().to_string());
        label.push_str("]/");
    }
    label.push_str(instance.node().as_str());
    label
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

/// What a rerun does with the business identifier of the run it repeats (`FR-018-040`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RerunBusinessId {
    /// The new run is bound to the identifier, unless another open execution holds it
    /// (`BUSINESS_ID_IN_USE`).
    #[default]
    Keep,
    /// The new run starts without one.
    Drop,
}

/// Why and from what a run is started again.
#[derive(Debug, Clone, Copy)]
pub struct Rerun<'a> {
    pub of: RunId,
    pub reason: &'a str,
    pub business: RerunBusinessId,
    /// Set when the run is a reset: the prefix of the source it re-emits (feature 018, `FR-018-037`).
    pub reset: Option<ResetStart<'a>>,
}

/// The prefix a reset re-emits under the run it creates, and what it delivers again after it.
#[derive(Clone, Copy)]
pub struct ResetStart<'a> {
    pub prefix: &'a RestoredPrefix,
    pub reapply: sagat_domain::execution::ResetReapply,
    /// What is delivered again to the new run after the point, in journal order, and what is not and why.
    pub plan: &'a [PlannedReapply],
    /// The justification the caller gave, which the register keeps.
    pub reason: &'a str,
    /// What became of the business identifier of the source.
    pub business: RerunBusinessId,
    /// What the request said to do with a source that had not ended: the register keeps what was asked.
    pub source_policy: ResetSourcePolicy,
    /// Whether the request acknowledged that steps run again.
    pub acknowledged: bool,
    /// Set when the run completes a reset that waited for its source (`CANCEL_SOURCE`).
    pub pending: Option<&'a PendingAdmission>,
}

impl std::fmt::Debug for ResetStart<'_> {
    /// The justification of the caller and the content of the plan are not printed.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResetStart")
            .field("source", &self.prefix.source)
            .field("sequence", &self.prefix.sequence())
            .field("reapply", &self.reapply)
            .field("source_policy", &self.source_policy)
            .field("pending", &self.pending.map(|pending| pending.reset))
            .finish_non_exhaustive()
    }
}

/// What a run started again does with the business identifier of the one it repeats: bound to it unless
/// another open execution holds it by now (then it is refused rather than started without the identifier it
/// was asked to keep), or without one. The source has ended, so its identifier may be taken over.
fn keep_business_id(business_id: Option<&BusinessId>, business: RerunBusinessId) -> StartOptions {
    match (business_id, business) {
        (Some(id), RerunBusinessId::Keep) => StartOptions {
            business: Some(BusinessStart {
                id: id.clone(),
                conflict: ConflictPolicy::Fail,
                reuse: ReusePolicy::AllowDuplicate,
                window: ReuseWindow::default(),
            }),
            start_delay: None,
        },
        _ => StartOptions::default(),
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
    lag_budget: Duration,
    input_blobs: Option<(
        crate::blobs::BlobService,
        Arc<dyn crate::blobs::RunInputChecker>,
    )>,
    /// Evaluates the declared Update an Update-with-Start carries; absent, that request is refused.
    declared_updates: Option<Arc<dyn DeclaredOperationEvaluator>>,
    /// Whether a reset may deliver signals and Updates again (`SAGAT_RESET_REAPPLY_ENABLED`, off by default).
    reset_reapply: bool,
    /// Whether a reset may cancel a source that has not ended and wait for it (`SAGAT_RESET_CANCEL_SOURCE_ENABLED`,
    /// off by default).
    reset_cancel_source: bool,
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
            declared_updates: None,
            reset_reapply: false,
            reset_cancel_source: false,
        }
    }

    /// Lets a reset deliver again the signals and the Updates of its source (`reapply` other than `NONE`,
    /// feature 018, `FR-018-039`). Off, such a request is refused `RESET_REAPPLY_NOT_AVAILABLE`.
    #[must_use]
    pub const fn with_reset_reapply(mut self, enabled: bool) -> Self {
        self.reset_reapply = enabled;
        self
    }

    /// Lets a reset cancel a source that has not ended and wait for it (`sourcePolicy` `CANCEL_SOURCE`, feature 018,
    /// `FR-018-038`). Off, such a request for a source that is open is refused `RESET_CANCEL_SOURCE_NOT_AVAILABLE`;
    /// a retry of one that was accepted is still answered, and the engine completes the ones that were.
    #[must_use]
    pub const fn with_reset_cancel_source(mut self, enabled: bool) -> Self {
        self.reset_cancel_source = enabled;
        self
    }

    /// Serves Update-with-Start: `evaluator` checks and evaluates the declared Update it carries.
    #[must_use]
    pub fn with_declared_updates(mut self, evaluator: Arc<dyn DeclaredOperationEvaluator>) -> Self {
        self.declared_updates = Some(evaluator);
        self
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

    /// Builds the chain command facade from the same store, clock and engine semantics.
    #[must_use]
    pub fn chain_commands(&self) -> crate::chain_progress::ChainCommandService<S, C, E>
    where
        S: Clone + crate::chain_progress::ChainCommandStore,
        <S as RunStore>::Tx: crate::chain_progress::ChainRunTx,
        C: Clone,
        E: Clone,
    {
        crate::chain_progress::ChainCommandService::new(
            self.store.clone(),
            self.clock.clone(),
            self.engine.clone(),
        )
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
        self.start_with(
            principal,
            request,
            StartOptions::default(),
            None,
            None,
            None,
            None,
            ctx,
        )
        .await
        .map(|started| started.run)
    }

    /// Starts a run with the options of feature 018 (a business identifier). The answer says whether
    /// the run already existed: with `USE_EXISTING` and an open holder nothing is created and the
    /// holder is returned.
    ///
    /// # Errors
    /// As [`Self::start`], and [`AppError::BusinessIdConflict`] when the identifier is held and the
    /// policy refuses (`BUSINESS_ID_IN_USE`, `BUSINESS_ID_REUSE_REJECTED`).
    pub async fn start_with_options(
        &self,
        principal: &Principal,
        request: &StartRun<'_>,
        options: StartOptions,
        ctx: &RequestContext,
    ) -> Result<Started, AppError> {
        self.start_with(principal, request, options, None, None, None, None, ctx)
            .await
    }

    /// Signal-with-Start (feature 018, `FR-018-024`): signals the open run of the business identifier,
    /// or, when there is none, creates a run and delivers the signal to it in the same transaction. If
    /// no step of the new run waits for the signal the whole request is rolled back; a signal is never
    /// buffered. The conflict policy of `business` is ignored: an open holder is always signalled.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `run:start` and `run:signal`; the errors of
    /// [`Self::start_with_options`]; [`AppError::SignalRefused`] (`NOT_WAITING` for an open holder
    /// that waits for nothing, `SIGNAL_NOT_ACCEPTED_AT_START` for a new run that would not accept
    /// it); [`AppError::NotAllowed`] for a payload that is not JSON of at most 256 KiB.
    pub async fn signal_with_start(
        &self,
        principal: &Principal,
        request: &StartRun<'_>,
        business: BusinessStart,
        signal: SignalAtStart<'_>,
        ctx: &RequestContext,
    ) -> Result<SignalStarted, AppError> {
        principal.authorize(Permission::RunSignal)?;
        let options = StartOptions {
            business: Some(BusinessStart {
                conflict: ConflictPolicy::UseExisting,
                ..business
            }),
            start_delay: None,
        };
        let started = self
            .start_with(
                principal,
                request,
                options,
                None,
                None,
                None,
                Some(WithStart::Signal(signal)),
                ctx,
            )
            .await?;
        Ok(SignalStarted {
            run: started.run,
            started: !started.existing,
            delivered_to: started.delivered_to,
        })
    }

    /// Update-with-Start (feature 018, `FR-018-025`): applies a declared Update to the open run of the
    /// business identifier (`USE_EXISTING`) or, when there is none, creates the run and applies it in
    /// the same transaction; with `FAIL` an open holder is a conflict. A business rejection (the
    /// validator said no, the expected revision is stale) leaves the run started with a stable
    /// `REJECTED` receipt. A failure before admission (an undeclared operation, arguments the
    /// declaration does not accept, a key reused with another request) creates nothing.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `run:start` and `run:update`; the errors of
    /// [`Self::start_with_options`]; [`AppError::DeclaredUpdate`] for what the declared Update refuses
    /// before admission; [`AppError::NotAllowed`] when the instance does not serve declared Updates or
    /// the request names no business identifier.
    pub async fn update_with_start(
        &self,
        principal: &Principal,
        request: &StartRun<'_>,
        options: StartOptions,
        update: UpdateAtStart<'_>,
        ctx: &RequestContext,
    ) -> Result<UpdateStarted, AppError> {
        principal.authorize(Permission::RunUpdate)?;
        if options.business.is_none() {
            return Err(AppError::NotAllowed(
                "an Update-with-Start request names the business identifier it acts on",
            ));
        }
        let started = self
            .start_with(
                principal,
                request,
                options,
                None,
                None,
                None,
                Some(WithStart::Update(update)),
                ctx,
            )
            .await?;
        let applied = started.update.ok_or(AppError::Internal)?;
        Ok(UpdateStarted {
            run: started.run,
            started: !started.existing,
            update: applied,
        })
    }

    /// Starts the run of a schedule's action (feature 018). The principal is the schedule
    /// (`system:schedule:<id>`) holding the role of whoever authorized it, so the run says who started
    /// it and `run:start` is the authority of that subject today. The run is admitted under the scope
    /// `schedule:{id}` and the key of the action, and the action row and the new state of the schedule
    /// commit with it.
    ///
    /// # Errors
    /// As [`Self::start`], and [`AppError::StateConflict`] when the schedule is not active, changed
    /// since the decision, the shard lease is no longer held, or the key of the action already has
    /// its run: another process acted on that time first, and this one did not.
    pub async fn start_scheduled(
        &self,
        principal: &Principal,
        request: &StartRun<'_>,
        admission: crate::schedules::ScheduleAdmission,
        ctx: &RequestContext,
    ) -> Result<RunRecord, AppError> {
        self.start_scheduled_under(principal, request, None, admission, ctx)
            .await
            .map(|started| started.run)
    }

    /// As [`Self::start_scheduled`], for an action whose run is started under a business identifier (the prefix of the
    /// schedule's action and the nominal time, feature 018 `FR-018-016`). The identifier is claimed in the creation
    /// transaction, after the schedule's row and before any run's (the lock order of ADR-0050 decision 7), with the
    /// policies the schedule names. The answer says whether a run was made (`existing` is false) or an open holder
    /// answered instead (`USE_EXISTING`): nothing was created and nothing of the action was written, so the caller
    /// records the action.
    ///
    /// # Errors
    /// As [`Self::start_scheduled`], and [`AppError::BusinessIdConflict`] when the identifier is held and the policy
    /// refuses (`BUSINESS_ID_IN_USE`, `BUSINESS_ID_REUSE_REJECTED`): nothing was written either, and the caller records
    /// the refusal as the decision of the action.
    pub async fn start_scheduled_under(
        &self,
        principal: &Principal,
        request: &StartRun<'_>,
        business: Option<BusinessStart>,
        admission: crate::schedules::ScheduleAdmission,
        ctx: &RequestContext,
    ) -> Result<Started, AppError> {
        self.start_with(
            principal,
            request,
            StartOptions {
                business,
                start_delay: None,
            },
            None,
            None,
            Some(admission),
            None,
            ctx,
        )
        .await
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
        self.start_with(
            principal,
            request,
            StartOptions::default(),
            None,
            Some(id),
            None,
            None,
            ctx,
        )
        .await
        .map(|started| started.run)
    }

    /// How a business identifier stands in a namespace: who holds it and how, `FR-018-022`.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `run:read`; [`AppError::NotFound`] for an identifier that is
    /// unbound, whose holder is gone, or that has been free for longer than its window.
    pub async fn business_binding(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        id: &BusinessId,
    ) -> Result<BusinessBinding, AppError> {
        principal.authorize(Permission::RunRead)?;
        self.store
            .business_binding(principal.tenant_id(), namespace, id)
            .await?
            .ok_or(AppError::NotFound)
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
        self.rerun_with(principal, id, reason, key, RerunBusinessId::Keep, ctx)
            .await
    }

    /// As [`Self::rerun`], and says what becomes of the business identifier of the run
    /// (`FR-018-040`): kept by default, or dropped.
    ///
    /// # Errors
    /// Those of [`Self::rerun`]; [`AppError::BusinessIdConflict`] when the identifier is kept and
    /// another open execution holds it.
    pub async fn rerun_with(
        &self,
        principal: &Principal,
        id: RunId,
        reason: &str,
        key: &str,
        business: RerunBusinessId,
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
        if original.info.origin.is_some() {
            return self
                .rerun_private(
                    principal,
                    &original,
                    &source,
                    Rerun {
                        of: id,
                        reason,
                        business,
                        reset: None,
                    },
                    key,
                    ctx,
                )
                .await;
        }
        // The new run is bound to the identifier of the one it repeats, which has ended and so may be
        // taken over, unless another open execution holds it by now: then the rerun is refused
        // rather than started without the identifier it was asked to keep.
        let options = keep_business_id(original.info.business_id.as_ref(), business);
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
            options,
            Some(Rerun {
                of: id,
                reason,
                business,
                reset: None,
            }),
            None,
            None,
            None,
            ctx,
        )
        .await
        .map(|started| started.run)
    }

    #[allow(clippy::too_many_arguments)]
    async fn start_with(
        &self,
        principal: &Principal,
        request: &StartRun<'_>,
        options: StartOptions,
        rerun: Option<Rerun<'_>>,
        input_blob: Option<sagat_domain::blobs::BlobId>,
        scheduled: Option<crate::schedules::ScheduleAdmission>,
        with: Option<WithStart<'_>>,
        ctx: &RequestContext,
    ) -> Result<Started, AppError> {
        principal.authorize(Permission::RunStart)?;
        if let Some(WithStart::Signal(signal)) = &with
            && (signal.payload.len() > MAX_PAYLOAD_BYTES
                || !self.engine.evaluator().is_json(signal.payload))
        {
            return Err(AppError::NotAllowed(
                "the signal payload must be JSON of at most 256 KiB",
            ));
        }
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
        if let Some(delay) = options.start_delay
            && !(Duration::from_millis(1)..=MAX_START_DELAY).contains(&delay)
        {
            return Err(AppError::InvalidStart {
                reason: "START_DELAY_INVALID",
                field: Some("/startDelayMs".to_owned()),
            });
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
                .replay_start_authorized(principal, workflow.id(), idempotency, ctx)
                .await?
            {
                return self.replayed_start(principal, &existing).await;
            }
        }
        let version_id = match request.version {
            Some(id) => id,
            None => workflow
                .active_version()
                .ok_or(AppError::StateConflict(NO_PUBLISHED_VERSION))?,
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
        let chain_deadline = definition
            .continuation_policy()
            .map(|policy| {
                time::Duration::try_from(policy.timeout())
                    .ok()
                    .and_then(|timeout| now.checked_add(timeout))
                    .ok_or(AppError::NotAllowed(
                        "the chain timeout exceeds the supported deadline",
                    ))
            })
            .transpose()?;
        let effective_deadline = match (request.deadline, chain_deadline) {
            (Some(requested), Some(chain)) => Some(requested.min(chain)),
            (requested, chain) => requested.or(chain),
        };
        let run_id = RunId::generate();
        let reset = rerun.and_then(|rerun| rerun.reset);
        let info = RunInfo {
            origin: None,
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
            deadline_at: effective_deadline,
            created_at: now,
            parent: None,
            depth: 0,
            root: run_id,
            business_id: None,
        };
        // The tick of a cron trigger's schedule that nobody asked for is the legacy cron pass's run: started and audited
        // by the engine, with the reason the legacy pass gives, so that whichever pass reaches the tick starts the same
        // run (feature 018, `FR-018-017`).
        let engine_tick = scheduled.as_ref().and_then(|admission| {
            admission
                .trigger
                .filter(|tick| tick.engine)
                .map(|tick| (tick, admission.nominal_at))
        });
        let audit = match (rerun, engine_tick) {
            (None, Some((tick, nominal))) => AuditRecord::engine(
                principal.tenant_id(),
                AuditAction::RunStarted,
                ResourceType::Run,
                run_id.to_string(),
                crate::schedule_triggers::tick_reason(tick.trigger, nominal),
                ctx.trace_id.clone(),
                now,
            ),
            (None, None) => AuditRecord::by(
                principal,
                AuditAction::RunStarted,
                ResourceType::Run,
                run_id.to_string(),
                ctx.trace_id.clone(),
                now,
            ),
            (
                Some(Rerun {
                    reset: Some(reset),
                    of,
                    reason,
                    ..
                }),
                _,
            ) => AuditRecord::by(
                principal,
                AuditAction::RunReset,
                ResourceType::Run,
                run_id.to_string(),
                ctx.trace_id.clone(),
                now,
            )
            .with_reason(format!(
                "reset from {of} at {}{}{}: {reason}",
                reset.prefix.sequence(),
                // the closed word of the policy, when something is to be delivered again; never content
                if reset.reapply == sagat_domain::execution::ResetReapply::None {
                    String::new()
                } else {
                    format!(" reapply={}", reset.reapply.as_str())
                },
                // the entry of the register that a pass completes: the join with the request that made it (an id)
                reset.pending.map_or_else(String::new, |pending| format!(
                    " completes reset {}",
                    pending.reset
                )),
            )),
            (Some(rerun), _) => AuditRecord::by(
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
        // The options join the fingerprint only when there are any, so the fingerprint of a start
        // that has none is what it always was and a retry made before an upgrade still replays.
        match &with {
            Some(WithStart::Signal(signal)) => {
                idempotency.fingerprint.extend(fingerprint(&[
                    b"run.signal_at_start",
                    signal.name.as_bytes(),
                    signal.payload,
                ]));
            }
            Some(WithStart::Update(update)) => {
                idempotency.fingerprint.extend(fingerprint(&[
                    b"run.update_at_start",
                    update.operation.as_str().as_bytes(),
                    update.update_key.as_str().as_bytes(),
                    update.arguments.canonical(),
                    &update.expected_state_revision.map_or_else(
                        || vec![0],
                        |revision| {
                            let mut bytes = vec![1];
                            bytes.extend(revision.to_be_bytes());
                            bytes
                        },
                    ),
                ]));
            }
            None => {}
        }
        if let Some(delay) = options.start_delay {
            idempotency.fingerprint.extend(fingerprint(&[
                b"run.start_delay",
                &delay.as_millis().to_be_bytes(),
            ]));
        }
        // A reset is the same request only for the same point of the same prefix with the same delivery after
        // it: the key of another point is another request, not a replay of this one.
        if let Some(reset) = rerun.and_then(|rerun| rerun.reset) {
            // The justification is part of the request (it is audited and kept in the register): the same key with
            // another one is another request, not a retry whose second justification is never written (D2010).
            idempotency.fingerprint.extend(fingerprint(&[
                b"run.reset",
                &reset.prefix.sequence().to_be_bytes(),
                reset.prefix.digest().as_bytes(),
                reset.reapply.as_str().as_bytes(),
                reset.reason.as_bytes(),
            ]));
        }
        if let Some(business) = &options.business {
            idempotency.fingerprint.extend(fingerprint(&[
                b"run.business",
                business.id.as_str().as_bytes(),
                business.conflict.as_str().as_bytes(),
                business.reuse.as_str().as_bytes(),
                &business.window.seconds().to_be_bytes(),
            ]));
        }
        // A rerun that keeps the identifier carries it in `options` (above), so the same key under `DROP`
        // is another request without a marker of its own; where the source has no identifier both
        // choices are the same request.
        if let Some(blob_idempotency) = blob_idempotency {
            idempotency = blob_idempotency;
        }
        if let Some(pending) = reset.and_then(|reset| reset.pending) {
            // the completion of a reset that waited is recognised by the reset, whatever else is sent: its key is
            // the platform's own, `reset:{id}` (the prefix is reserved), given by `complete_reset`, so nobody retries
            // it, and a replay is another engine that completed the entry first
            idempotency = Idempotency {
                scope: format!("run.start:{}", namespace.id()),
                key: request.key.to_owned(),
                fingerprint: fingerprint(&[
                    b"run.reset.completion",
                    pending.reset.to_string().as_bytes(),
                    request.input,
                ]),
            };
        }
        if let Some(admission) = &scheduled {
            // an action is recognised by its schedule and its nominal time, whatever the caller sends; the time of a
            // cron trigger's schedule is recognised by its trigger and its tick, as feature 004 has it, so that the
            // legacy pass and this one never start it twice (the input is not part of it: a tick is one run)
            idempotency = match admission.tick() {
                Some(tick) => Idempotency {
                    scope: tick.scope,
                    key: tick.key,
                    fingerprint: tick.fingerprint,
                },
                None => Idempotency {
                    scope: format!("schedule:{}", admission.schedule),
                    key: admission.run_key(),
                    fingerprint: fingerprint(&[
                        b"schedule.action",
                        admission.schedule.to_string().as_bytes(),
                        admission.run_key().as_bytes(),
                        workflow.id().to_string().as_bytes(),
                        version.version.id().to_string().as_bytes(),
                        request.input,
                    ]),
                },
            };
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
                            .replay_start_authorized(principal, workflow.id(), &idempotency, ctx)
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
        let pending_completion = reset.is_some_and(|reset| reset.pending.is_some());
        let limits = RunLimits::of(&self.store, principal.tenant_id(), namespace.id())
            .await
            .map_err(|error| store_failure(pending_completion, error))?;
        self.cache
            .initial_declared_state(&version.document, &input)
            .map_err(|error| {
                tracing::debug!(%error, version = %version.version.id(), "declared initial state rejected the run input");
                AppError::NotAllowed(
                    "the run input does not produce a valid declared initial state",
                )
            })?;
        let new = NewRun {
            retained: None,
            limits,
            info,
            definition: Arc::clone(&definition),
            input,
            input_blob: input_binding,
            audit,
            idempotency,
            schedule: scheduled,
            business: options.business,
            reset_sequence: reset.map(|reset| reset.prefix.sequence()),
            pending_reset: reset.and_then(|reset| reset.pending).cloned(),
        };
        let scheduled_start = new.schedule.is_some();
        let by_dispatcher = engine_tick.is_some();
        let start_receipt = new.idempotency.clone();
        // The instant the entry is held until, to the microsecond: the journal and the column hold
        // the same number, so a replay of the one is the other.
        let held_until_us = options
            .start_delay
            .map(|delay| {
                now.checked_add(time::Duration::try_from(delay).map_err(|_| AppError::Internal)?)
                    .and_then(|at| i64::try_from(at.unix_timestamp_nanos() / 1_000).ok())
                    .ok_or(AppError::Internal)
            })
            .transpose()?;
        let created = match self.store.create(new).await {
            Ok(created) => created,
            Err(error) => {
                self.audit_database_denial(principal, workflow.id(), &error, ctx)
                    .await;
                return Err(store_failure(pending_completion, error));
            }
        };
        self.finish_start(
            principal,
            run_id,
            created,
            scheduled_start,
            by_dispatcher,
            held_until_us,
            with.map(|with| (with, &start_receipt)),
            reset,
            ctx,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn finish_start(
        &self,
        principal: &Principal,
        run_id: RunId,
        created: Admitted<S::Tx>,
        scheduled: bool,
        by_dispatcher: bool,
        held_until_us: Option<i64>,
        with: Option<(WithStart<'_>, &Idempotency)>,
        reset: Option<ResetStart<'_>>,
        ctx: &RequestContext,
    ) -> Result<Started, AppError> {
        match created {
            // the key of a scheduled time is the platform's own: nobody retries it, so a replay is
            // another process that acted on that time first
            Admitted::Replayed(_) if scheduled => Err(AppError::StateConflict(
                "the schedule already acted on that time",
            )),
            // the entry of a pending reset was completed by another engine that committed first
            Admitted::Replayed(_) if reset.is_some_and(|reset| reset.pending.is_some()) => {
                Err(AppError::StateConflict(RESET_ALREADY_COMPLETED))
            }
            Admitted::Replayed(existing) => {
                let mut started = self.known_start(principal, &existing, false).await?;
                match with {
                    Some((WithStart::Signal(signal), start)) => {
                        // what the first request delivered, from the receipt it left with the run
                        let (delivered, served) = self
                            .delivered_before(principal, started.run.info.run_id, signal, start)
                            .await?;
                        started.delivered_to = delivered;
                        started.existing = served;
                    }
                    Some((WithStart::Update(update), start)) => {
                        // the receipt the first request left answers again, whatever the run is now
                        let (applied, served) = self
                            .update_in_own_transaction(
                                principal,
                                started.run.info.run_id,
                                started.run.info.namespace_id,
                                update,
                                HolderReceipt::Read(start),
                                ctx,
                            )
                            .await?;
                        started.update = Some(applied);
                        started.existing = served;
                    }
                    None => {}
                }
                Ok(started)
            }
            // the holder of the business identifier, returned and nothing created
            Admitted::Existing(holder) => {
                // The holder of a continuation chain is its initial run; the work is in the segment
                // that is current now, which is where a signal or an Update must go.
                let holder_run = holder.parse::<RunId>().map_err(|_| AppError::Internal)?;
                let target = self
                    .store
                    .current_segment(principal.tenant_id(), holder_run)
                    .await?;
                let mut started = self
                    .known_start(principal, &target.to_string(), true)
                    .await?;
                match with {
                    Some((WithStart::Signal(signal), start)) => {
                        // an open holder is signalled exactly as `signalRun` would
                        let (delivered, _) = self
                            .signal_with_receipt(
                                principal,
                                target,
                                signal.name,
                                signal.payload,
                                &start.key,
                                HolderReceipt::Keep(start),
                                WaitVia::SignalWithStart,
                                ctx,
                            )
                            .await
                            .map_err(|error| match error {
                                AppError::StateConflict(NOT_WAITING_FOR_SIGNAL) => {
                                    AppError::SignalRefused {
                                        reason: "NOT_WAITING",
                                    }
                                }
                                other => other,
                            })?;
                        started.delivered_to = delivered;
                    }
                    Some((WithStart::Update(update), start)) => {
                        // an open holder gets the Update exactly as `updateRun` would apply it
                        let (applied, _) = self
                            .update_in_own_transaction(
                                principal,
                                target,
                                started.run.info.namespace_id,
                                update,
                                HolderReceipt::Keep(start),
                                ctx,
                            )
                            .await?;
                        started.update = Some(applied);
                    }
                    None => {}
                }
                Ok(started)
            }
            Admitted::Created(mut tx) => {
                let pending_completion = reset.is_some_and(|reset| reset.pending.is_some());
                let mut reapplied: Vec<ReapplyEntry> = Vec::new();
                // the journal of the tick of a cron trigger says what the legacy pass's does: the dispatcher did it
                let effects = Effects::by(
                    if by_dispatcher {
                        Actor::System("event-dispatcher")
                    } else {
                        Actor::Principal(principal.subject().as_str().to_owned())
                    },
                    ctx.trace_id.clone(),
                );
                if let Some(reset) = reset {
                    // A reset that would deliver again is refused here, where a run would be created, and not
                    // before the replay of the request: a retry of a request that succeeded must be answered from
                    // the register whatever the setting is now. Nothing has been written: the transaction is dropped.
                    // The completion of a request that was accepted is not asked again: the setting belongs to the
                    // control plane, the engine has none (D1990), and what was accepted stands (D1997).
                    if reset.pending.is_none()
                        && let Err(refusal) = self.reapply_available(reset.reapply)
                    {
                        drop(tx);
                        return Err(refusal);
                    }
                    // the events of the source up to the point are written as they were, and the run goes on
                    // from there: the reset is recorded and the successors of the point are released
                    tx.restore_prefix(reset.prefix)
                        .await
                        .map_err(|error| store_failure(pending_completion, error))?;
                    self.engine
                        .run(&mut tx, effects, |def, state, now| {
                            let progress = execution::record_reset(
                                def,
                                state,
                                now,
                                reset.prefix.source,
                                reset.prefix.sequence(),
                                reset.prefix.digest(),
                                reset.reapply,
                            )?;
                            Ok(((), progress))
                        })
                        .await
                        .map_err(|error| match error {
                            EngineError::Store(store) => store_failure(pending_completion, store),
                            other => engine_error(other),
                        })?;
                    // what the source received after the point is delivered again, in its order, in this
                    // transaction: a failure that is not an answer of the new run drops it, and with it the run
                    let reset_id = reset
                        .pending
                        .map_or_else(execution::ResetId::generate, |pending| pending.reset);
                    let report = self
                        .reapply(&mut tx, principal, run_id, reset_id, reset.plan, ctx)
                        .await?;
                    reapplied.clone_from(&report);
                    // the register says it in the same transaction, so a run made by a reset always has its entry
                    tx.register_reset(&ResetRegistration {
                        id: reset_id,
                        source: reset.prefix.source,
                        sequence: reset.prefix.sequence(),
                        prefix_digest: reset.prefix.digest(),
                        reapply: reset.reapply,
                        business: reset.business,
                        source_policy: reset.source_policy,
                        acknowledged: reset.acknowledged,
                        actor: principal.subject().as_str(),
                        reason: reset.reason,
                        report: &report,
                        completes_pending: reset.pending.is_some(),
                    })
                    .await
                    .map_err(|error| store_failure(pending_completion, error))?;
                } else {
                    self.engine
                        .run(&mut tx, effects, |def, state, now| {
                            let (started, progress) = match held_until_us {
                                Some(until_us) => {
                                    execution::start_run_delayed(def, run_id, now, until_us)?
                                }
                                None => execution::start_run(def, run_id, now)?,
                            };
                            *state = started;
                            Ok(((), progress))
                        })
                        .await
                        .map_err(engine_error)?;
                }
                let mut delivered_to = Vec::new();
                let mut update_applied = None;
                if let Some((WithStart::Update(update), _)) = with {
                    // the Update goes in the creation transaction, after the initial reduction: a
                    // refusal before admission drops the transaction and nothing of the run exists; a
                    // business rejection commits with the run, its receipt stable
                    let evaluator = self.declared_evaluator()?;
                    let command = self.declared_update(
                        principal,
                        tx.info().namespace_id,
                        run_id,
                        update,
                        // the clock of the run's own events, the database's
                        tx.now(),
                        ctx,
                    );
                    update_applied = Some(tx.apply_declared_update(&command, evaluator).await?);
                }
                if let Some((WithStart::Signal(signal), start)) = with {
                    // the signal goes in the creation transaction, to every step that waits for it
                    // after the initial reduction; if none does, nothing is created at all
                    let idempotency = Idempotency {
                        scope: format!("run.signal:{run_id}"),
                        key: start.key.clone(),
                        fingerprint: fingerprint(&[signal.name.as_bytes(), signal.payload]),
                    };
                    delivered_to = match self
                        .deliver_signal(
                            &mut tx,
                            principal,
                            run_id,
                            signal.name,
                            signal.payload,
                            idempotency,
                            WaitVia::SignalWithStart,
                            ctx,
                        )
                        .await
                    {
                        Ok(woken) => woken,
                        Err(AppError::StateConflict(NOT_WAITING_FOR_SIGNAL)) => {
                            drop(tx);
                            return Err(AppError::SignalRefused {
                                reason: "SIGNAL_NOT_ACCEPTED_AT_START",
                            });
                        }
                        Err(other) => return Err(other),
                    };
                }
                tx.commit()
                    .await
                    .map_err(|error| store_failure(pending_completion, error))?;
                // counted once it is committed: a creation that was dropped delivered nothing
                count_reapply(&reapplied);
                let mut started = self
                    .known_start(principal, &run_id.to_string(), false)
                    .await?;
                started.delivered_to = delivered_to;
                started.update = update_applied;
                Ok(started)
            }
        }
    }

    /// The evaluator of declared Updates, or the refusal of a service that does not serve them.
    fn declared_evaluator(&self) -> Result<&dyn DeclaredOperationEvaluator, AppError> {
        // A server that was not wired for it, not a caller's mistake: an internal error, said once.
        self.declared_updates.as_deref().ok_or_else(|| {
            tracing::error!("a declared Update was asked of a run service without an evaluator");
            AppError::Internal
        })
    }

    /// The declared Update addressed to `run`, with the audit and the outbox notification that
    /// commit with it.
    fn declared_update(
        &self,
        principal: &Principal,
        namespace: sagat_domain::identity::NamespaceId,
        run: RunId,
        update: UpdateAtStart<'_>,
        occurred_at: OffsetDateTime,
        ctx: &RequestContext,
    ) -> DeclaredUpdate {
        let audit = AuditRecord::by(
            principal,
            AuditAction::RunUpdated,
            ResourceType::Run,
            run.to_string(),
            ctx.trace_id.clone(),
            occurred_at,
        );
        let request = UpdateRequest::new(
            OperationScope::new(principal.tenant_id(), namespace, OperationAddress::Run(run)),
            run,
            update.operation.clone(),
            update.update_key.clone(),
            Digest::new([0; 32]),
            update.expected_state_revision,
            principal.subject().clone(),
        );
        let command = DeclaredUpdate::new(
            request,
            update.arguments.clone(),
            audit,
            uuid::Uuid::now_v7(),
            occurred_at,
        );
        match update.batch_job {
            Some(job) => command.with_batch_job(job),
            None => command,
        }
    }

    /// Applies the Update to a run that exists, in a transaction of its own: the open holder of a
    /// business identifier, or the run a retry of the same request finds already started (the receipt
    /// the first request left answers again).
    async fn update_in_own_transaction(
        &self,
        principal: &Principal,
        run: RunId,
        namespace: sagat_domain::identity::NamespaceId,
        update: UpdateAtStart<'_>,
        receipt: HolderReceipt<'_>,
        ctx: &RequestContext,
    ) -> Result<(AppliedInTx, bool), AppError> {
        let evaluator = self.declared_evaluator()?;
        let Some(mut tx) = self
            .store
            .begin(principal.tenant_id(), run, Lock::Wait)
            .await?
        else {
            return Err(AppError::Internal);
        };
        let command = self.declared_update(principal, namespace, run, update, tx.now(), ctx);
        let applied = tx.apply_declared_update(&command, evaluator).await?;
        let served = match receipt {
            HolderReceipt::None => false,
            HolderReceipt::Keep(start) => {
                tx.remember_start_served(start, &served_marker(start), run)
                    .await?;
                true
            }
            HolderReceipt::Read(start) => tx.replay(&served_marker(start)).await?.is_some(),
        };
        tx.commit().await?;
        Ok((applied, served))
    }

    async fn audit_database_denial(
        &self,
        principal: &Principal,
        workflow: WorkflowId,
        error: &StoreError,
        ctx: &RequestContext,
    ) {
        if !matches!(
            error,
            StoreError::DatabaseUnavailable | StoreError::DatabaseLimit
        ) {
            return;
        }
        let audit = AuditRecord::by(
            principal,
            AuditAction::DatabaseDenied,
            ResourceType::Workflow,
            workflow.to_string(),
            ctx.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason("database run admission refused");
        if let Err(error) = RunStore::record_denial(&self.store, &audit).await {
            tracing::error!(%error, "failed to record database denial");
        }
    }

    async fn replay_start_authorized(
        &self,
        principal: &Principal,
        workflow: WorkflowId,
        idempotency: &Idempotency,
        ctx: &RequestContext,
    ) -> Result<Option<String>, AppError> {
        let result = self
            .store
            .replay_start(
                principal.tenant_id(),
                principal.subject().as_str(),
                idempotency,
            )
            .await;
        if let Err(error) = &result {
            self.audit_database_denial(principal, workflow, error, ctx)
                .await;
        }
        result.map_err(Into::into)
    }

    async fn replayed_start(
        &self,
        principal: &Principal,
        existing: &str,
    ) -> Result<Started, AppError> {
        self.known_start(principal, existing, false).await
    }

    /// The run a start came to without creating one, as the answer of the start.
    async fn known_start(
        &self,
        principal: &Principal,
        id: &str,
        existing: bool,
    ) -> Result<Started, AppError> {
        let id = id.parse::<RunId>().map_err(|_| AppError::Internal)?;
        let run = self
            .store
            .get(principal.tenant_id(), id)
            .await?
            .ok_or(AppError::Internal)?;
        Ok(Started {
            run,
            existing,
            delivered_to: Vec::new(),
            update: None,
        })
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
        if operation == RunOperation::Cancel {
            tx.latch_chain_cancel().await?;
        }
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
        self.review_instance(
            principal,
            id,
            &ExecutionInstanceId::root(node.clone()),
            action,
            reason,
            expected_version,
            key,
            ctx,
        )
        .await
    }

    /// The same decision about a step of an iteration of a collection or loop: the compensation of
    /// one iteration that ran out of attempts, or a member whose outcome nobody could establish.
    /// The instance names the step exactly (its scope path and its node); the permissions, the
    /// justification, the single winner of two racing decisions and the audit are those of
    /// [`Self::review`].
    ///
    /// # Errors
    /// As [`Self::review`].
    #[allow(clippy::too_many_arguments)]
    pub async fn review_instance(
        &self,
        principal: &Principal,
        id: RunId,
        instance: &ExecutionInstanceId,
        action: &ReviewAction,
        reason: &str,
        expected_version: Option<u64>,
        key: &str,
        ctx: &RequestContext,
    ) -> Result<RunRecord, AppError> {
        let node = instance.node();
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
        let label = instance_label(instance);
        let idempotency = Idempotency {
            scope: format!("run.review:{id}"),
            key: key.to_owned(),
            fingerprint: fingerprint(&[
                label.as_bytes(),
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
        if tx.parts().1.instance(instance).is_none() {
            return Err(AppError::NotFound);
        }
        let version = tx.parts().1.version();
        if expected_version.is_some_and(|expected| expected != version) {
            return Err(AppError::RevisionConflict { current: version });
        }
        // The domain checks retry safety under this run lock for every role and task kind.
        // SagaOverride authorizes outcome-bearing decisions, never a blind repeat.
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
                format!("{id}/{label}"),
                ctx.trace_id.clone(),
                now,
            )
            .with_reason(audited),
        );
        effects.idempotency = Some((idempotency, action.name()));
        let action = action.clone();
        if instance.path().is_root() {
            let step = node.clone();
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
        } else {
            let step = instance.clone();
            self.engine
                .run_addressed(&mut tx, Vec::new(), effects, move |def, state, now| {
                    let progress = match &action {
                        ReviewAction::Retry => {
                            execution::retry_review_instance(def, state, &step, now)?
                        }
                        ReviewAction::Hold => {
                            execution::mark_for_review_instance(def, state, &step, now)?
                        }
                        ReviewAction::Skip => {
                            execution::skip_review_instance(def, state, &step, now)?
                        }
                        ReviewAction::Resolve(outcome) => execution::resolve_review_instance(
                            def,
                            state,
                            &step,
                            outcome.clone(),
                            now,
                        )?,
                    };
                    Ok(((), progress))
                })
                .await
                .map_err(engine_error)?;
        }
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
        self.signal_with_receipt(
            principal,
            id,
            name,
            payload,
            key,
            HolderReceipt::None,
            WaitVia::Signal,
            ctx,
        )
        .await
        .map(|(woken, _)| woken)
    }

    /// [`Self::signal`], and, when an existing holder of a business identifier serves a with-start
    /// request, the receipts of that request kept in the same transaction ([`HolderReceipt`]).
    #[allow(clippy::too_many_arguments)]
    async fn signal_with_receipt(
        &self,
        principal: &Principal,
        id: RunId,
        name: &str,
        payload: &[u8],
        key: &str,
        receipt: HolderReceipt<'_>,
        via: WaitVia,
        ctx: &RequestContext,
    ) -> Result<(Vec<NodeId>, bool), AppError> {
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
        let woken = match tx.replay(&idempotency).await? {
            Some(delivered) => delivered
                .split(',')
                .filter(|n| !n.is_empty())
                .filter_map(|n| NodeId::parse(n).ok())
                .collect(),
            None => {
                self.deliver_signal(&mut tx, principal, id, name, payload, idempotency, via, ctx)
                    .await?
            }
        };
        let served = match receipt {
            HolderReceipt::None => false,
            HolderReceipt::Keep(start) => {
                tx.remember_start_served(start, &served_marker(start), id)
                    .await?;
                true
            }
            HolderReceipt::Read(start) => tx.replay(&served_marker(start)).await?.is_some(),
        };
        tx.commit().await?;
        Ok((woken, served))
    }

    /// The receipt a with-start request left with its run: the nodes its signal woke.
    async fn delivered_before(
        &self,
        principal: &Principal,
        id: RunId,
        signal: SignalAtStart<'_>,
        start: &Idempotency,
    ) -> Result<(Vec<NodeId>, bool), AppError> {
        let idempotency = Idempotency {
            scope: format!("run.signal:{id}"),
            key: start.key.clone(),
            fingerprint: fingerprint(&[signal.name.as_bytes(), signal.payload]),
        };
        let Some(mut tx) = self
            .store
            .begin(principal.tenant_id(), id, Lock::Wait)
            .await?
        else {
            return Err(AppError::Internal);
        };
        // The receipt of the signal is written with the start's, in one transaction: a start that
        // is replayed and has none is a broken invariant, not an empty answer.
        let Some(delivered) = tx.replay(&idempotency).await? else {
            tracing::error!("a replayed with-start request has no signal receipt");
            return Err(AppError::Internal);
        };
        let served = tx.replay(&served_marker(start)).await?.is_some();
        Ok((
            delivered
                .split(',')
                .filter(|n| !n.is_empty())
                .filter_map(|n| NodeId::parse(n).ok())
                .collect(),
            served,
        ))
    }

    /// Delivers the signal to every step of the run in `tx` that waits for it, under the receipt
    /// `idempotency`, and returns the nodes woken. The caller commits (or, for a creation, rolls back).
    ///
    /// # Errors
    /// [`AppError::StateConflict`] when no step waits for the signal, and the failures of the run.
    #[allow(clippy::too_many_arguments)]
    async fn deliver_signal(
        &self,
        tx: &mut S::Tx,
        principal: &Principal,
        id: RunId,
        name: &str,
        payload: &[u8],
        idempotency: Idempotency,
        via: WaitVia,
        ctx: &RequestContext,
    ) -> Result<Vec<NodeId>, AppError> {
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
            return Err(AppError::StateConflict(NOT_WAITING_FOR_SIGNAL));
        }
        // What a run retains is bounded (every lease carries it): a signal that would exceed it is
        // refused instead of stored.
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
            .run(tx, effects, move |def, state, now| {
                let mut events = Vec::new();
                let mut pending = None;
                for node in &targets {
                    let progress = execution::satisfy_wait(def, state, node, now, Some(via))?;
                    events.extend(progress.events);
                    pending = pending.or(progress.pending);
                }
                Ok(((), Progress { events, pending }))
            })
            .await
            .map_err(engine_error)?;
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

/// What a failure of the store comes to when the run being made completes a reset that waited for its source (feature
/// 018, D2007). The conversion that a client is told turns an unavailable database connection and a connection limit
/// into `NotFound` and `NotAllowed`, and a missing reference into `NotFound`; for the pass of the engine those are not
/// answers of the admission but failures to try again, and written as the outcome of the reset they would end it for
/// good after the source has been cancelled. Everything else (a quota, a business identifier held, a lost lease) is
/// converted as for every other caller.
pub(crate) fn store_failure(pending_completion: bool, error: StoreError) -> AppError {
    if pending_completion
        && matches!(
            error,
            StoreError::DatabaseUnavailable
                | StoreError::DatabaseLimit
                | StoreError::MissingReference
                | StoreError::RevealGrantRequired
        )
    {
        return AppError::Unavailable;
    }
    error.into()
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
        ExecutionError::OutcomeUnresolved(_) => AppError::OutcomeUnresolved,
        ExecutionError::StaleAttempt { .. }
        | ExecutionError::IllegalTransition { .. }
        | ExecutionError::IllegalInstanceTransition { .. } => {
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

impl<S, C, E> crate::schedule_firing::ScheduledStarts for RunService<S, C, E>
where
    S: RunStore + DefinitionStore,
    C: Clock,
    E: ExpressionEvaluator,
{
    async fn start_scheduled(
        &self,
        principal: &Principal,
        request: &crate::schedule_firing::ScheduledStart<'_>,
        admission: crate::schedules::ScheduleAdmission,
        ctx: &RequestContext,
    ) -> Result<crate::schedule_firing::ScheduledRun, AppError> {
        let key = admission.run_key();
        let started = RunService::start_scheduled_under(
            self,
            principal,
            &StartRun {
                namespace: request.namespace,
                workflow: request.workflow,
                version: request.version,
                input: request.input,
                correlation_id: None,
                deadline: None,
                key: &key,
            },
            request.business.clone(),
            admission,
            ctx,
        )
        .await?;
        Ok(if started.existing {
            crate::schedule_firing::ScheduledRun::Existing(started.run.info.run_id)
        } else {
            crate::schedule_firing::ScheduledRun::Started(started.run.info.run_id)
        })
    }

    async fn cancel_run(
        &self,
        principal: &Principal,
        run: uuid::Uuid,
        reason: &str,
        key: &str,
        ctx: &RequestContext,
    ) -> Result<(), AppError> {
        RunService::operate(
            self,
            principal,
            RunId::new(run),
            RunOperation::Cancel,
            reason,
            None,
            key,
            ctx,
        )
        .await
        .map(|_| ())
    }
}
