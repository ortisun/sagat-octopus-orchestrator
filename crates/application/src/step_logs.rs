//! The step-log ingest of design 041 section 2 (task T041-6): `AppendStepLogs`, one batch of the records of a leased
//! attempt that is still running, stored in one transaction of its own beside the journal (ADR-0084).
//!
//! The rules are here and in `sagat_domain::step_logs`; the port [`StepLogStore`] opens a [`StepLogTx`] whose methods
//! are the reads and writes of the steps T1 to T10 of the design, one method each, and hold no rule. The pre-checks run
//! before any statement: P1 the caller's permission, P2 the deployment switch, P4 the batch's bounds and P5 the
//! normalisation, redaction, cut and size of every record (P3, the tenant's ticket, is the gRPC handler's, task T041-7).
//!
//! The control flow (D041-050): `LEASE` (T2) ends the transaction at once; `SESSION`, `CAPTURE_OFF` (T3), `REMOVED` and
//! `FINAL_RECORDED` (T4) are noted and the transaction skips to T7, so that a caller whose binding or membership is gone
//! learns only `AUTHORITY`, whatever else is true; a noted answer or `AUTHORITY` rolls back, so nothing is stored or
//! counted. A lock timeout (`55P03`, the 200 ms of the transaction or a refill's wait) and a refusal of the quota-lease
//! charge at T8 roll back and answer `RETRY_LATER`; the worker sends the same batch again.
//!
//! The content of a record never reaches a log, a span, a metric or an error (FR-041-007 (d)): the request types print
//! counts only, and every store error is mapped without the database's text.

use std::future::Future;
use std::time::Instant;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::binding::{ActivityPattern, first_uncovered};
use sagat_domain::execution::{ExecutionInstanceId, Fence, RunId};
use sagat_domain::identity::{NamespaceId, Permission, Principal, Role, TenantId};
use sagat_domain::step_logs::{
    AttemptCeilings, BatchInvalid, DropReason, Drops, FieldValue, MAX_TEXT_BYTES, MarkerReason,
    Plan, PlanVerdict, REDACTED, RecordShape, SizedRecord, StepLogLevel, StopReason,
    StreamCounters, TenantAllowance, canonical_fields, cut, normalise, plan_batch, validate_batch,
};
use sagat_telemetry::{StepLogBatchOutcome, StepLogQuotaRefusalCause, StepLogRedactionRule};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::clock::Clock;
use crate::entropy::Token;
use crate::error::{AppError, StoreError};

/// The protocol minor a session must have been accepted at for its batches to be stored (protocol 1.7).
pub const STEP_LOG_PROTOCOL_MINOR: u32 = 7;

/// What `RETRY_LATER` asks the worker to wait before it sends the same batch again: a lock timeout, a quota-lease
/// refusal, or no ticket free (design section 1; D041-061).
pub const RETRY_AFTER_MS: u32 = 200;

/// The SQLSTATE of a lock that was not granted in time (`lock_timeout`, a refill's budget, or no wait slot free).
const LOCK_NOT_AVAILABLE: &str = "55P03";

// ---------------------------------------------------------------------------------- the request

/// One record as the worker sent it (`StepLogRecord`). Its `Debug` prints its sequence and sizes only: the text is
/// what a step's code logged (FR-041-007 (d)).
#[derive(Clone, PartialEq)]
pub struct StepLogRecordIn {
    pub sequence: u64,
    /// `None` when the wire value names no level (unspecified or unknown): refused by P4.
    pub level: Option<StepLogLevel>,
    pub worker_time: Option<OffsetDateTime>,
    pub message: String,
    /// Empty when none.
    pub exception: String,
    pub fields: Vec<(String, FieldValue)>,
    /// The worker already cut the message, the exception or the fields.
    pub truncated: bool,
}

impl std::fmt::Debug for StepLogRecordIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StepLogRecordIn")
            .field("sequence", &self.sequence)
            .field("level", &self.level)
            .field("message_bytes", &self.message.len())
            .field("exception_bytes", &self.exception.len())
            .field("fields", &self.fields.len())
            .field("truncated", &self.truncated)
            .finish_non_exhaustive()
    }
}

/// One `AppendStepLogs` call: the attempt, named and fenced as a heartbeat names and fences it, the session that
/// registered the worker, and the records. Its `Debug` hides the tokens and the text.
#[derive(Clone)]
pub struct StepLogBatch {
    /// The session id `RegisterWorkerResponse` returned.
    pub worker_id: Uuid,
    pub session_token: String,
    pub run_id: RunId,
    pub step_id: Uuid,
    pub instance: ExecutionInstanceId,
    pub fence: Fence,
    pub lease_token: String,
    pub records: Vec<StepLogRecordIn>,
    /// The encoded size of the request, for P4's bound of 192 KiB.
    pub encoded_len: usize,
}

impl std::fmt::Debug for StepLogBatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StepLogBatch")
            .field("worker_id", &self.worker_id)
            .field("run_id", &self.run_id)
            .field("step_id", &self.step_id)
            .field("instance", &self.instance)
            .field("fence", &self.fence)
            .field("records", &self.records.len())
            .field("encoded_len", &self.encoded_len)
            .finish_non_exhaustive()
    }
}

/// What the worker does next (`StepLogVerdict`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepLogVerdict {
    Continue,
    /// A ceiling was met: send no more records; the report still carries the final mark.
    FinalOnly,
    /// Send nothing more for this attempt.
    StopAttempt(StopReason),
    /// Send nothing more on this session.
    StopCapture(StopReason),
    /// Nothing stored; send the same batch again after this many milliseconds.
    RetryLater {
        after_ms: u32,
    },
}

/// The answer to a batch (`AppendStepLogsResponse`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchAnswer {
    pub verdict: StepLogVerdict,
    /// The stream's highest worker sequence now known to the server; only with `Continue`, `FinalOnly` and
    /// `StopAttempt(FinalRecorded)`, else 0.
    pub stored_through_sequence: u64,
}

impl BatchAnswer {
    const fn stop(reason: StopReason) -> Self {
        let verdict = if reason.stops_capture() {
            StepLogVerdict::StopCapture(reason)
        } else {
            StepLogVerdict::StopAttempt(reason)
        };
        Self {
            verdict,
            stored_through_sequence: 0,
        }
    }

    const fn retry_later() -> Self {
        Self {
            verdict: StepLogVerdict::RetryLater {
                after_ms: RETRY_AFTER_MS,
            },
            stored_through_sequence: 0,
        }
    }

    const fn outcome(&self, stored_lines: u64) -> StepLogBatchOutcome {
        match self.verdict {
            StepLogVerdict::Continue if stored_lines > 0 => StepLogBatchOutcome::Stored,
            StepLogVerdict::Continue => StepLogBatchOutcome::Duplicate,
            StepLogVerdict::FinalOnly => StepLogBatchOutcome::FinalOnly,
            StepLogVerdict::StopAttempt(_) => StepLogBatchOutcome::StopAttempt,
            StepLogVerdict::StopCapture(_) => StepLogBatchOutcome::StopCapture,
            StepLogVerdict::RetryLater { .. } => StepLogBatchOutcome::RetryLater,
        }
    }
}

// ---------------------------------------------------------------------------------- the port

/// The attempt a batch names, as T2 matches it. `lease_token_hash` is the hash of the presented lease token. Its
/// `Debug` prints no hash.
#[derive(Clone, Copy)]
pub struct AttemptRef<'a> {
    pub tenant: TenantId,
    pub run_id: RunId,
    pub step_id: Uuid,
    pub instance: &'a ExecutionInstanceId,
    pub fence: Fence,
    pub lease_token_hash: &'a [u8],
}

impl std::fmt::Debug for AttemptRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AttemptRef")
            .field("tenant", &self.tenant)
            .field("run_id", &self.run_id)
            .field("step_id", &self.step_id)
            .field("instance", &self.instance)
            .field("fence", &self.fence)
            .finish_non_exhaustive()
    }
}

/// What T2 read of the `steps` row and its run, without a lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseRead {
    pub namespace: NamespaceId,
    /// The activity the step's lease is for (`steps.activity`); `None` for a step that names none.
    pub activity: Option<String>,
    pub cancel_requested: bool,
    pub run_finished: bool,
    /// The run is stopping (`runs.stop_reason` set).
    pub run_stopping: bool,
}

/// The session T3 checks: the session id and the hash of the presented session token. Its `Debug` prints neither
/// the hash nor the subject.
#[derive(Clone, Copy)]
pub struct SessionRef<'a> {
    pub tenant: TenantId,
    pub namespace: NamespaceId,
    pub worker_id: Uuid,
    pub subject: &'a str,
    pub token_hash: &'a [u8],
}

impl std::fmt::Debug for SessionRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionRef")
            .field("tenant", &self.tenant)
            .field("namespace", &self.namespace)
            .field("worker_id", &self.worker_id)
            .finish_non_exhaustive()
    }
}

/// The three namespace resources of data-model.md §6 in force for a batch, read under their policy fences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepLogPolicy {
    /// `step_log_attempt_records`: 0 turns capture off in the namespace.
    pub attempt_records: u64,
    pub attempt_bytes: u64,
    pub retention_days: u64,
}

/// The attempt's stream as T4 creates or locks it.
#[derive(Debug, Clone, Copy)]
pub struct StreamRef<'a> {
    pub tenant: TenantId,
    pub namespace: NamespaceId,
    pub run_id: RunId,
    pub step_id: Uuid,
    pub instance: &'a ExecutionInstanceId,
    pub fence: Fence,
    /// For a new stream: the namespace's retention read at T3.
    pub retention_days: u64,
}

/// What T4 read of the stream row it created or locked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamRead {
    pub stream_id: i64,
    pub counters: StreamCounters,
    /// Its removal began (purge mark or delete).
    pub removed: bool,
    /// `expires_at` has passed.
    pub expired: bool,
    /// The attempt's carry recorded its final mark.
    pub final_recorded: bool,
}

/// The unlocked reads of T7 (OD-2026-11-16-7): the binding's patterns and the caller's membership role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityRead {
    pub patterns: Vec<String>,
    pub role: Option<Role>,
}

/// One line T8 inserts, after P5.
#[derive(Clone, PartialEq, Eq)]
pub struct LineRow {
    pub sequence: u64,
    pub level: StepLogLevel,
    pub worker_time: Option<OffsetDateTime>,
    pub message: String,
    pub exception: Option<String>,
    pub fields: Option<String>,
    pub truncated: bool,
}

impl std::fmt::Debug for LineRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LineRow")
            .field("sequence", &self.sequence)
            .field("level", &self.level)
            .field("truncated", &self.truncated)
            .finish_non_exhaustive()
    }
}

/// What T8 inserts: the kept lines and the ceiling marker, one statement.
#[derive(Debug, Clone)]
pub struct InsertRows<'a> {
    pub tenant: TenantId,
    pub stream_id: i64,
    pub run_id: RunId,
    pub lines: Vec<&'a LineRow>,
    pub marker: Option<(u64, MarkerReason)>,
}

/// One row T8 inserted, as its `RETURNING` read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InsertedRow {
    pub sequence: u64,
    /// The UTF-8 octets of its message, exception and fields, as `sagat_quota_measure` measures them.
    pub text_bytes: u64,
    pub truncated: bool,
    pub level: StepLogLevel,
    /// A ceiling marker rather than a worker line.
    pub marker: bool,
}

/// Why the quota-lease charge of T8 refused the insert (its constraint name).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaRefusal {
    /// `tenant_step_log_quota_exceeded`: the tenant is at its `tenant_step_log_bytes` limit.
    Limit,
    /// `tenant_step_log_quota_held`: the room left is in quota leases that open transactions hold.
    Held,
}

/// What T8 did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertOutcome {
    Inserted(Vec<InsertedRow>),
    /// The charge raised; the transaction is to be rolled back and nothing was stored.
    Refused(QuotaRefusal),
}

/// The counters T9 writes on the stream row, worked out from the row T4 locked and the rows T8 returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CounterUpdate {
    pub tenant: TenantId,
    pub stream_id: i64,
    pub counters: StreamCounters,
    /// T8 inserted at least one row: `last_stored_xid` is raised to this transaction's id.
    pub stored: bool,
}

/// The step-log storage port (data-model.md §8). Implemented by `sagat-infra-postgres`.
pub trait StepLogStore: Send + Sync {
    type Tx: StepLogTx;

    /// Opens the transaction of one batch (`begin_safely`); its lock timeout (200 ms by default) is set by T1.
    fn begin(&self) -> impl Future<Output = Result<Self::Tx, StoreError>> + Send;

    /// Whether `run` belongs to another tenant (no lock): the cross-tenant audit of a `LEASE` answer.
    fn run_exists_outside(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Records the audit of a denial, outside any transaction of the ingest.
    fn record_denial(
        &self,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}

/// The transaction of one batch: the steps T1 to T10 of design section 2, one method each, holding no rule.
pub trait StepLogTx: Send {
    /// T1: the transaction's lock timeout, its id, assigned before the lease is read, and the store's wait slots *Q*
    /// for the refills of this transaction (decision D041-137), in one statement.
    fn assign_transaction_id(&mut self) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// T2: the `steps` row of the attempt and its run, read without a lock; `None` when no row holds this lease.
    fn lease(
        &mut self,
        attempt: &AttemptRef<'_>,
    ) -> impl Future<Output = Result<Option<LeaseRead>, StoreError>> + Send;

    /// T3, first half: whether the session is live, of the lease's namespace and the caller's subject, holds the
    /// presented token and was accepted at 1.7 or later. No lock.
    fn session(
        &mut self,
        session: &SessionRef<'_>,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// T3, second half: the six policy fences of the three resources taken shared (tenant then namespace, in the
    /// order of `QuotaResource`) and the attempt's stream lock (L1, then L1b), in one statement, then the three
    /// values in another.
    fn policies(
        &mut self,
        stream: &StreamRef<'_>,
    ) -> impl Future<Output = Result<StepLogPolicy, StoreError>> + Send;

    /// T4: the stream row, created or locked to the commit.
    fn stream(
        &mut self,
        stream: &StreamRef<'_>,
    ) -> impl Future<Output = Result<StreamRead, StoreError>> + Send;

    /// T6, the tenant's half: `sagat_quota_step_log_allowance(tenant)`, without a lock.
    fn tenant_allowance(
        &mut self,
        tenant: TenantId,
    ) -> impl Future<Output = Result<TenantAllowance, StoreError>> + Send;

    /// T7, the binding's patterns of `subject` in `namespace`, without the binding lock.
    fn binding_patterns(
        &mut self,
        tenant: TenantId,
        namespace: NamespaceId,
        subject: &str,
    ) -> impl Future<Output = Result<Vec<String>, StoreError>> + Send;

    /// T7, the caller's membership role, a plain read (no `FOR SHARE`, no namespace secrets lock).
    fn membership_role(
        &mut self,
        tenant: TenantId,
        subject: &str,
    ) -> impl Future<Output = Result<Option<Role>, StoreError>> + Send;

    /// T8: the kept lines and the marker, at most once each; the statement trigger charges them to the writer's
    /// quota lease, and its refusal is answered, not raised.
    fn insert(
        &mut self,
        rows: &InsertRows<'_>,
    ) -> impl Future<Output = Result<InsertOutcome, StoreError>> + Send;

    /// T9: the counters on the stream row, `last_at`, `expires_at` and, when T8 stored a row, `last_stored_xid`.
    fn record_counters(
        &mut self,
        update: &CounterUpdate,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// T10.
    fn commit(self) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Ends the transaction without storing anything.
    fn rollback(self) -> impl Future<Output = Result<(), StoreError>> + Send;
}

// ---------------------------------------------------------------------------------- P5

/// One record after P5: normalised, scrubbed, cut, its fields in canonical JSON, sized as the quota measures it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRecord {
    pub row: LineRow,
    /// The UTF-8 octets of the message, the exception and the fields (the attempt ceilings' measure; the tenant
    /// quota charges 128 more per row).
    pub text_bytes: u64,
    /// Parts a credential shape was scrubbed from (`step_log_redactions_total{rule="shape"}`).
    pub shapes: u64,
    /// Field values replaced because their key is a sensitive name (`{rule="field_name"}`).
    pub field_names: u64,
}

/// `text` with the credential shapes of both server scrubbers replaced by `[REDACTED]`, nothing cut, and whether
/// anything was replaced (FR-041-007 (c)): the script logs' shapes (`redact_credentials`) and the telemetry's
/// (`scrub_shapes_with`), through their public entry points, so that no second list of shapes exists.
#[must_use]
pub fn scrub(text: &str) -> (String, bool) {
    let credentials = sagat_domain::scripts::redact_credentials(text);
    let shaped = sagat_telemetry::scrub_shapes_with(&credentials, REDACTED).into_owned();
    let changed = shaped != text;
    (shaped, changed)
}

fn scrubbed_text(text: &str, max: usize) -> (String, bool, bool) {
    let (clean, changed) = scrub(&normalise(text));
    let (kept, was_cut) = cut(&clean, max);
    (kept.to_owned(), changed, was_cut)
}

/// P5 for one record (FR-041-003, FR-041-007 (c)): NUL becomes U+FFFD; the message, the exception, every field key and
/// every string field value are scrubbed with the server's shapes; the value of any type of a field whose key is a
/// sensitive name becomes the string `[REDACTED]`; the message and the exception are cut to 8 KiB at a character
/// boundary; the fields become their canonical JSON, kept up to 32 keys and 2 KiB; and the record is sized exactly as
/// `sagat_quota_measure` measures it. The record must have passed P4 (its level is known).
#[must_use]
pub fn prepare(record: &StepLogRecordIn) -> PreparedRecord {
    let (message, message_scrubbed, message_cut) = scrubbed_text(&record.message, MAX_TEXT_BYTES);
    let (exception, exception_scrubbed, exception_cut) = if record.exception.is_empty() {
        (None, false, false)
    } else {
        let (text, scrubbed, was_cut) = scrubbed_text(&record.exception, MAX_TEXT_BYTES);
        (Some(text), scrubbed, was_cut)
    };
    let mut shapes = u64::from(message_scrubbed) + u64::from(exception_scrubbed);
    let mut field_names = 0;
    let fields: Vec<(String, FieldValue)> = record
        .fields
        .iter()
        .map(|(key, value)| {
            let (clean_key, key_scrubbed) = scrub(&normalise(key));
            shapes += u64::from(key_scrubbed);
            let sensitive = sagat_telemetry::is_sensitive_name(key)
                || sagat_telemetry::is_sensitive_name(&clean_key);
            let value = if sensitive {
                field_names += 1;
                FieldValue::String(REDACTED.to_owned())
            } else if let FieldValue::String(text) = value {
                let (clean, scrubbed) = scrub(&normalise(text));
                shapes += u64::from(scrubbed);
                FieldValue::String(clean)
            } else {
                value.clone()
            };
            (clean_key, value)
        })
        .collect();
    let (fields, fields_dropped) = canonical_fields(&fields);
    let text_bytes = (message.len()
        + exception.as_ref().map_or(0, String::len)
        + fields.as_ref().map_or(0, String::len)) as u64;
    PreparedRecord {
        row: LineRow {
            sequence: record.sequence,
            level: record.level.unwrap_or(StepLogLevel::Info),
            worker_time: record.worker_time,
            message,
            exception,
            fields,
            truncated: record.truncated || message_cut || exception_cut || fields_dropped,
        },
        text_bytes,
        shapes,
        field_names,
    }
}

// ---------------------------------------------------------------------------------- the stop rule

/// Whether a report of the attempt would pass its fence now (OD-2026-11-16-11): the step not cancel-requested, the run
/// not finished, and the run not stopping unless the node is a compensation or a reconciliation node. T2 applies the
/// rule `attempt_authority_fence` applies to a report.
///
/// Pending decision (D041-146 (1)): this is the rule of `attempt_authority_fence`
/// (`crates/application/src/worker.rs`) for a report that does not ask for the cancellation path; design 041 makes
/// it one domain function, `report_fence_allows` in `crates/domain/src/execution/state.rs`, which both call. Those two
/// files belong to pull request #260 until it merges, so the function lands with the merge of develop and this one
/// is then replaced by it.
#[must_use]
pub fn report_fence_allows(lease: &LeaseRead, instance: &ExecutionInstanceId) -> bool {
    !lease.cancel_requested
        && !lease.run_finished
        && (!lease.run_stopping || instance.node().is_derived())
}

// ---------------------------------------------------------------------------------- the service

/// Why a transaction ended without an answer of its own.
enum TxFailure {
    /// `55P03`: the 200 ms lock timeout, or a refill's wait past its budget or with no wait slot free.
    LockTimeout,
    /// The quota-lease charge of T8 raised.
    Quota(QuotaRefusal),
    Store(StoreError),
}

impl From<StoreError> for TxFailure {
    fn from(error: StoreError) -> Self {
        if error.sqlstate() == Some(LOCK_NOT_AVAILABLE) {
            Self::LockTimeout
        } else {
            Self::Store(error)
        }
    }
}

/// What a transaction that reached its end answered, and what it stored.
struct Transacted {
    answer: BatchAnswer,
    /// The rows T8 inserted, when the transaction committed.
    stored: Vec<InsertedRow>,
    /// The drop counts the commit added to the stream, per reason, after saturation.
    dropped: Vec<(DropReason, u64)>,
    /// The run of the batch belongs to another tenant: audited after the transaction.
    cross_tenant: bool,
}

impl Transacted {
    const fn answer(answer: BatchAnswer) -> Self {
        Self {
            answer,
            stored: Vec::new(),
            dropped: Vec::new(),
            cross_tenant: false,
        }
    }
}

/// The ingest service (design section 2).
pub struct StepLogIngest<S, C> {
    store: S,
    clock: C,
    /// `SAGAT_STEP_LOGS_ENABLED` (P2).
    enabled: bool,
}

impl<S: StepLogStore, C: Clock> StepLogIngest<S, C> {
    pub const fn new(store: S, clock: C, enabled: bool) -> Self {
        Self {
            store,
            clock,
            enabled,
        }
    }

    pub const fn store(&self) -> &S {
        &self.store
    }

    /// Whether the deployment switch lets capture run (P2).
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    /// Stores one batch and answers what the worker does next.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] when the caller may not execute work or is a runner or database-runner identity (P1);
    /// [`AppError::NotAllowed`] naming the bound a malformed batch breaks (P4); a store failure other than a lock
    /// timeout or a quota-lease refusal, which are answered `RETRY_LATER`.
    pub async fn append(
        &self,
        principal: &Principal,
        batch: &StepLogBatch,
    ) -> Result<BatchAnswer, AppError> {
        let started = Instant::now();
        let result = self.append_counted(principal, batch).await;
        metrics::histogram!("step_log_ingest_seconds").record(started.elapsed().as_secs_f64());
        result
    }

    async fn append_counted(
        &self,
        principal: &Principal,
        batch: &StepLogBatch,
    ) -> Result<BatchAnswer, AppError> {
        authorize(principal)?;
        if !self.enabled {
            let answer = BatchAnswer::stop(StopReason::CaptureOff);
            count_batch(answer.outcome(0));
            return Ok(answer);
        }
        if let Err(invalid) = validate(batch) {
            count_batch(StepLogBatchOutcome::Invalid);
            return Err(AppError::NotAllowed(invalid_message(invalid)));
        }
        let prepared: Vec<PreparedRecord> = batch.records.iter().map(prepare).collect();
        match self.transact(principal, batch, &prepared).await {
            Ok(done) => {
                if done.cross_tenant {
                    self.audit_cross_tenant(principal, batch.run_id).await;
                }
                count_stored(&done.stored, &prepared, &done.dropped);
                let lines = done.stored.iter().filter(|row| !row.marker).count() as u64;
                count_batch(done.answer.outcome(lines));
                Ok(done.answer)
            }
            Err(TxFailure::LockTimeout) => {
                metrics::counter!("step_log_lock_timeouts_total").increment(1);
                count_batch(StepLogBatchOutcome::RetryLater);
                Ok(BatchAnswer::retry_later())
            }
            Err(TxFailure::Quota(refusal)) => {
                let cause = match refusal {
                    QuotaRefusal::Limit => StepLogQuotaRefusalCause::Limit,
                    QuotaRefusal::Held => StepLogQuotaRefusalCause::Held,
                };
                metrics::counter!("step_log_quota_refusals_total", "cause" => cause.as_str())
                    .increment(1);
                metrics::counter!("step_log_quota_retries_total").increment(1);
                count_batch(StepLogBatchOutcome::RetryLater);
                Ok(BatchAnswer::retry_later())
            }
            Err(TxFailure::Store(error)) => Err(error.into()),
        }
    }

    /// T1 to T10. A failure after `begin` drops the transaction, which rolls it back.
    async fn transact(
        &self,
        principal: &Principal,
        batch: &StepLogBatch,
        prepared: &[PreparedRecord],
    ) -> Result<Transacted, TxFailure> {
        let tenant = principal.tenant_id();
        let subject = principal.subject().as_str();
        let lease_token_hash = Token::hash_of(&batch.lease_token);
        let mut tx = self.store.begin().await?;
        tx.assign_transaction_id().await?;
        // T2
        let attempt = AttemptRef {
            tenant,
            run_id: batch.run_id,
            step_id: batch.step_id,
            instance: &batch.instance,
            fence: batch.fence,
            lease_token_hash: &lease_token_hash,
        };
        let lease = match tx.lease(&attempt).await? {
            Some(lease) if report_fence_allows(&lease, &batch.instance) => lease,
            found => {
                tx.rollback().await?;
                let cross_tenant = found.is_none()
                    && self
                        .store
                        .run_exists_outside(tenant, batch.run_id)
                        .await
                        .unwrap_or(false);
                return Ok(Transacted {
                    cross_tenant,
                    ..Transacted::answer(BatchAnswer::stop(StopReason::Lease))
                });
            }
        };
        // T3 to T6, unless an answer is noted on the way; T7 runs either way.
        let session_token_hash = Token::hash_of(&batch.session_token);
        let stream_ref = StreamRef {
            tenant,
            namespace: lease.namespace,
            run_id: batch.run_id,
            step_id: batch.step_id,
            instance: &batch.instance,
            fence: batch.fence,
            retention_days: 0,
        };
        let staged = self
            .stage(
                &mut tx,
                &SessionRef {
                    tenant,
                    namespace: lease.namespace,
                    worker_id: batch.worker_id,
                    subject,
                    token_hash: &session_token_hash,
                },
                stream_ref,
                prepared,
            )
            .await;
        let staged = match staged {
            Ok(staged) => staged,
            Err(failure) => {
                // A lock timeout or a store error at T3 or T4 still answers a revoked caller `AUTHORITY` (D041-050):
                // the transaction is dropped, which rolls it back, and T7's two reads run in one of their own.
                drop(tx);
                return self
                    .authority_before(tenant, subject, &lease, failure)
                    .await;
            }
        };
        // T7
        let patterns = tx
            .binding_patterns(tenant, lease.namespace, subject)
            .await?;
        let role = tx.membership_role(tenant, subject).await?;
        if !authorized(&AuthorityRead { patterns, role }, lease.activity.as_deref()) {
            tx.rollback().await?;
            return Ok(Transacted::answer(BatchAnswer::stop(StopReason::Authority)));
        }
        let (stream, plan) = match staged {
            Staged::Noted(answer) => {
                tx.rollback().await?;
                return Ok(Transacted::answer(answer));
            }
            Staged::Planned { stream, plan } => (stream, plan),
        };
        // T8
        let rows = InsertRows {
            tenant,
            stream_id: stream.stream_id,
            run_id: batch.run_id,
            lines: plan
                .keep
                .iter()
                .map(|&index| &prepared[index].row)
                .collect(),
            marker: plan.marker,
        };
        let inserted = match tx.insert(&rows).await? {
            InsertOutcome::Inserted(inserted) => inserted,
            InsertOutcome::Refused(refusal) => return Err(TxFailure::Quota(refusal)),
        };
        // T9
        let counters = counters_after(&stream.counters, &plan, &inserted);
        tx.record_counters(&CounterUpdate {
            tenant,
            stream_id: stream.stream_id,
            counters,
            stored: !inserted.is_empty(),
        })
        .await?;
        // T10
        tx.commit().await?;
        let verdict = match plan.verdict {
            PlanVerdict::Continue => StepLogVerdict::Continue,
            PlanVerdict::FinalOnly => StepLogVerdict::FinalOnly,
        };
        Ok(Transacted {
            answer: BatchAnswer {
                verdict,
                stored_through_sequence: counters.last_sequence,
            },
            dropped: saturated_drops(&stream.counters, &counters, &plan.drops),
            stored: inserted,
            cross_tenant: false,
        })
    }

    /// T3 to T6: the session, the policies and the stream lock, the stream row, the plan and the tenant allowance. An
    /// answer met on the way is noted and the rest is skipped.
    async fn stage(
        &self,
        tx: &mut S::Tx,
        session: &SessionRef<'_>,
        stream_ref: StreamRef<'_>,
        prepared: &[PreparedRecord],
    ) -> Result<Staged, TxFailure> {
        if !tx.session(session).await? {
            return Ok(Staged::Noted(BatchAnswer::stop(StopReason::Session)));
        }
        let policy = tx.policies(&stream_ref).await?;
        if policy.attempt_records == 0 {
            return Ok(Staged::Noted(BatchAnswer::stop(StopReason::CaptureOff)));
        }
        let stream = tx
            .stream(&StreamRef {
                retention_days: policy.retention_days,
                ..stream_ref
            })
            .await?;
        if stream.removed || stream.expired {
            return Ok(Staged::Noted(BatchAnswer::stop(StopReason::Removed)));
        }
        if stream.final_recorded {
            return Ok(Staged::Noted(BatchAnswer {
                verdict: StepLogVerdict::StopAttempt(StopReason::FinalRecorded),
                stored_through_sequence: stream.counters.last_sequence,
            }));
        }
        let tenant = tx.tenant_allowance(session.tenant).await?;
        let sized: Vec<SizedRecord> = prepared
            .iter()
            .map(|record| SizedRecord {
                sequence: record.row.sequence,
                text_bytes: record.text_bytes,
                truncated: record.row.truncated,
            })
            .collect();
        let plan = plan_batch(
            &stream.counters,
            AttemptCeilings {
                records: policy.attempt_records,
                bytes: policy.attempt_bytes,
            },
            tenant,
            &sized,
            None,
        );
        Ok(Staged::Planned {
            stream,
            plan: Box::new(plan),
        })
    }

    /// T7 for a batch whose T3 or T4 failed: `AUTHORITY` when the binding or the membership no longer covers the step,
    /// else the failure (`RETRY_LATER` for a lock timeout). The reads take no lock; their transaction is rolled back.
    async fn authority_before(
        &self,
        tenant: TenantId,
        subject: &str,
        lease: &LeaseRead,
        failure: TxFailure,
    ) -> Result<Transacted, TxFailure> {
        let mut tx = self.store.begin().await?;
        let patterns = tx
            .binding_patterns(tenant, lease.namespace, subject)
            .await?;
        let role = tx.membership_role(tenant, subject).await?;
        tx.rollback().await?;
        if authorized(&AuthorityRead { patterns, role }, lease.activity.as_deref()) {
            Err(failure)
        } else {
            Ok(Transacted::answer(BatchAnswer::stop(StopReason::Authority)))
        }
    }

    async fn audit_cross_tenant(&self, principal: &Principal, run: RunId) {
        let record = AuditRecord::by(
            principal,
            AuditAction::CrossTenantAccessDenied,
            ResourceType::Run,
            run.to_string(),
            None,
            self.clock.now(),
        )
        .with_reason("worker call on a resource not accessible from this tenant");
        if let Err(error) = self.store.record_denial(&record).await {
            tracing::error!(%error, action = %record.action(), "failed to record audit entry");
        }
    }
}

/// What T3 to T6 left for T7.
enum Staged {
    /// An answer met on the way: given if T7 passes, the transaction rolled back either way.
    Noted(BatchAnswer),
    Planned {
        stream: StreamRead,
        plan: Box<Plan>,
    },
}

/// P1: `worker:execute`, and not a runner or database-runner identity, whose tokens never log a step's lines.
fn authorize(principal: &Principal) -> Result<(), AppError> {
    principal
        .authorize(Permission::WorkerExecute)
        .map_err(|_| AppError::Forbidden {
            permission: Permission::WorkerExecute,
        })?;
    if matches!(principal.role(), Role::Runner | Role::DatabaseRunner) {
        return Err(AppError::Forbidden {
            permission: Permission::WorkerExecute,
        });
    }
    Ok(())
}

/// P4. The 192 KiB bound is checked against the larger of the size the transport reports and what the records
/// hold (their text, field keys and string values, 8 bytes for any other value), so a wrong report cannot let a larger
/// batch reach P5 (the security review of task T041-6).
fn validate(batch: &StepLogBatch) -> Result<(), BatchInvalid> {
    let shapes: Vec<RecordShape> = batch
        .records
        .iter()
        .map(|record| RecordShape {
            sequence: record.sequence,
            level: record.level,
        })
        .collect();
    let held: usize = batch
        .records
        .iter()
        .map(|record| {
            record.message.len()
                + record.exception.len()
                + record
                    .fields
                    .iter()
                    .map(|(key, value)| {
                        key.len()
                            + match value {
                                FieldValue::String(text) => text.len(),
                                _ => 8,
                            }
                    })
                    .sum::<usize>()
        })
        .sum();
    validate_batch(&shapes, batch.encoded_len.max(held))
}

/// The fixed text of a refused batch: it names the bound, never a value.
const fn invalid_message(invalid: BatchInvalid) -> &'static str {
    match invalid {
        BatchInvalid::Empty => "a step-log batch holds no record",
        BatchInvalid::TooManyRecords => "a step-log batch holds more than 500 records",
        BatchInvalid::TooLarge => "a step-log batch is larger than 192 KiB",
        BatchInvalid::SequenceOutOfRange => "a step-log sequence is outside 1 to 2^62-1",
        BatchInvalid::NotIncreasing => "step-log sequences are not strictly increasing",
        BatchInvalid::UnknownLevel => "a step-log record has an unknown level",
    }
}

/// T7's rule: the binding covers the lease's activity (`first_uncovered`), and the caller is a member whose role may
/// execute work and is neither runner nor database-runner (the role rule of `current_worker_authorized`).
fn authorized(read: &AuthorityRead, activity: Option<&str>) -> bool {
    let Some(activity) = activity else {
        return false;
    };
    let Ok(patterns) = read
        .patterns
        .iter()
        .map(|text| ActivityPattern::parse(text))
        .collect::<Result<Vec<_>, _>>()
    else {
        return false;
    };
    let covered = first_uncovered(&patterns, &[activity.to_owned()]).is_none();
    let role_may = read.role.is_some_and(|role| {
        !matches!(role, Role::Runner | Role::DatabaseRunner)
            && role.permissions().contains(&Permission::WorkerExecute)
    });
    covered && role_may
}

/// The stream's counters after T8: the plan's, with the records, bytes, truncated records and marker taken from the
/// rows the insert returned, never from the batch.
fn counters_after(
    before: &StreamCounters,
    plan: &Plan,
    inserted: &[InsertedRow],
) -> StreamCounters {
    let lines = inserted.iter().filter(|row| !row.marker);
    let markers = inserted.iter().filter(|row| row.marker).count() as u64;
    StreamCounters {
        records: before.records + lines.clone().count() as u64,
        bytes: before.bytes + lines.clone().map(|row| row.text_bytes).sum::<u64>(),
        truncated_records: before.truncated_records
            + lines.filter(|row| row.truncated).count() as u64,
        next_marker_sequence: before.next_marker_sequence + markers,
        ..plan.after
    }
}

/// The change of the stream's drop counts, split by reason (D041-140 (7)): the saturated rise of
/// `dropped_by_worker` is `worker_gap`; the saturated rise of `dropped_by_server` is shared among the server's
/// reasons in the order of [`DropReason::ALL`], so that the shared counter never rises by more than the stream did.
fn saturated_drops(
    before: &StreamCounters,
    after: &StreamCounters,
    drops: &Drops,
) -> Vec<(DropReason, u64)> {
    let mut out = Vec::new();
    let by_worker = after
        .dropped_by_worker
        .saturating_sub(before.dropped_by_worker);
    if by_worker > 0 {
        out.push((DropReason::WorkerGap, by_worker));
    }
    let mut by_server = after
        .dropped_by_server
        .saturating_sub(before.dropped_by_server);
    for (reason, count) in drops.iter() {
        if reason == DropReason::WorkerGap || by_server == 0 {
            continue;
        }
        let share = count.min(by_server);
        by_server -= share;
        out.push((reason, share));
    }
    out
}

fn count_batch(outcome: StepLogBatchOutcome) {
    metrics::counter!("step_log_batches_total", "outcome" => outcome.as_str()).increment(1);
}

/// The metrics of the rows a committed batch stored and the drops it added.
fn count_stored(
    stored: &[InsertedRow],
    prepared: &[PreparedRecord],
    dropped: &[(DropReason, u64)],
) {
    let mut bytes = 0;
    let mut truncated = 0;
    let (mut shapes, mut field_names) = (0, 0);
    for row in stored.iter().filter(|row| !row.marker) {
        metrics::counter!("step_log_lines_stored_total", "level" => row.level.as_str())
            .increment(1);
        bytes += row.text_bytes;
        truncated += u64::from(row.truncated);
        if let Some(record) = prepared
            .iter()
            .find(|record| record.row.sequence == row.sequence)
        {
            shapes += record.shapes;
            field_names += record.field_names;
        }
    }
    if bytes > 0 {
        metrics::counter!("step_log_bytes_stored_total").increment(bytes);
    }
    if truncated > 0 {
        metrics::counter!("step_log_lines_truncated_total").increment(truncated);
    }
    for (rule, count) in [
        (StepLogRedactionRule::Shape, shapes),
        (StepLogRedactionRule::FieldName, field_names),
    ] {
        if count > 0 {
            metrics::counter!("step_log_redactions_total", "rule" => rule.as_str())
                .increment(count);
        }
    }
    for (reason, count) in dropped {
        metrics::counter!("step_log_lines_dropped_total", "reason" => reason.as_str())
            .increment(*count);
    }
}

#[cfg(test)]
mod tests {
    use sagat_domain::execution::ExecutionInstanceId;
    use sagat_domain::step_logs::{
        CeilingReason, FieldValue, MAX_FIELDS_BYTES, StepLogLevel, StreamCounters,
    };
    use sagat_domain::workflow::NodeId;

    use super::{LeaseRead, StepLogRecordIn, prepare, report_fence_allows, saturated_drops, scrub};

    fn record(message: &str) -> StepLogRecordIn {
        StepLogRecordIn {
            sequence: 1,
            level: Some(StepLogLevel::Info),
            worker_time: None,
            message: message.to_owned(),
            exception: String::new(),
            fields: Vec::new(),
            truncated: false,
        }
    }

    /// A message is scrubbed by both scrubbers' shapes, then cut at 8 KiB on a character boundary, and its size is its
    /// UTF-8 octets as stored.
    #[test]
    fn a_message_is_scrubbed_before_it_is_cut_and_sized() {
        let aws = "AKIAABCDEFGHIJKLMNOP";
        let done = prepare(&record(&format!("key {aws} end")));
        assert_eq!(done.row.message, "key [REDACTED] end");
        assert_eq!(done.shapes, 1);
        assert_eq!(done.text_bytes, "key [REDACTED] end".len() as u64);
        // 8,191 ASCII bytes and a two-byte character: the character does not fit and the cut keeps 8,191 bytes.
        let long = format!("{}é", "a".repeat(8_191));
        let done = prepare(&record(&long));
        assert_eq!(done.row.message.len(), 8_191);
        assert!(done.row.truncated);
        assert_eq!(done.text_bytes, 8_191);
    }

    /// NUL becomes U+FFFD (three octets) before the size is taken.
    #[test]
    fn a_nul_becomes_the_replacement_character() {
        let done = prepare(&record("a\0b"));
        assert_eq!(done.row.message, "a\u{FFFD}b");
        assert_eq!(done.text_bytes, 5);
    }

    /// A field whose key is a sensitive name has its value replaced whatever its type; a credential-shaped key and
    /// a credential-shaped string value are scrubbed; the fields are canonical JSON.
    #[test]
    fn a_sensitive_field_name_hides_a_value_of_any_type() {
        let mut input = record("m");
        input.fields = vec![
            ("password".into(), FieldValue::Int(1234)),
            ("api_key".into(), FieldValue::Bool(true)),
            ("token".into(), FieldValue::Double(1.5)),
            (
                "note".into(),
                FieldValue::String("Bearer abcdefgh12345678".into()),
            ),
        ];
        let done = prepare(&input);
        assert_eq!(
            done.row.fields.as_deref(),
            Some(
                "{\"api_key\":\"[REDACTED]\",\"note\":\"Bearer [REDACTED]\",\"password\":\"[REDACTED]\",\"token\":\"[REDACTED]\"}"
            )
        );
        assert_eq!(done.field_names, 3);
        assert_eq!(done.shapes, 1);
        assert_eq!(
            done.text_bytes,
            1 + done.row.fields.as_ref().map_or(0, String::len) as u64
        );
    }

    /// Fields past 2 KiB are dropped and the record marked truncated; an empty exception is stored as none.
    #[test]
    fn fields_past_their_bound_are_dropped_and_marked() {
        let mut input = record("");
        input.fields = (0..40)
            .map(|i| (format!("k{i:02}"), FieldValue::String("v".repeat(100))))
            .collect();
        let done = prepare(&input);
        assert!(done.row.truncated);
        assert!(
            done.row
                .fields
                .as_ref()
                .is_some_and(|f| f.len() <= MAX_FIELDS_BYTES)
        );
        assert_eq!(done.row.exception, None);
    }

    /// `scrub` reports whether anything changed and cuts nothing.
    #[test]
    fn scrub_cuts_nothing_and_says_whether_it_changed() {
        let long = "word ".repeat(1_000);
        assert_eq!(scrub(&long), (long.clone(), false));
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIabc\n-----END RSA PRIVATE KEY-----";
        assert_eq!(scrub(pem), ("[REDACTED]".to_owned(), true));
    }

    fn lease(cancel_requested: bool, run_finished: bool, run_stopping: bool) -> LeaseRead {
        LeaseRead {
            namespace: sagat_domain::identity::NamespaceId::new(uuid::Uuid::nil()),
            activity: Some("act".into()),
            cancel_requested,
            run_finished,
            run_stopping,
        }
    }

    /// The stop rule of a report (OD-2026-11-16-11): a forward step of a stopping run is refused and its compensation
    /// passes; a cancel-requested step and a finished run are refused.
    #[test]
    fn the_report_fence_refuses_what_a_report_would_be_refused() {
        let forward = ExecutionInstanceId::root(NodeId::parse("charge").expect("a node"));
        let compensation = forward.compensation();
        assert!(compensation.node().is_derived());
        assert!(report_fence_allows(&lease(false, false, false), &forward));
        assert!(!report_fence_allows(&lease(false, false, true), &forward));
        assert!(report_fence_allows(
            &lease(false, false, true),
            &compensation
        ));
        assert!(!report_fence_allows(&lease(true, false, false), &forward));
        assert!(!report_fence_allows(
            &lease(false, true, false),
            &compensation
        ));
    }

    /// The security review of task T041-6: P4's bound of 192 KiB is checked against what the records hold as well
    /// as against the size the transport reports, so a wrong `encoded_len` cannot let a larger batch through.
    #[test]
    fn a_batch_larger_than_its_bound_is_refused_whatever_size_it_reports() {
        let records: Vec<StepLogRecordIn> = (1..=25)
            .map(|sequence| StepLogRecordIn {
                sequence,
                ..record(&"x".repeat(8_000))
            })
            .collect();
        let batch = super::StepLogBatch {
            worker_id: uuid::Uuid::nil(),
            session_token: String::new(),
            run_id: sagat_domain::execution::RunId::new(uuid::Uuid::nil()),
            step_id: uuid::Uuid::nil(),
            instance: ExecutionInstanceId::root(NodeId::parse("a").expect("a node")),
            fence: sagat_domain::execution::Fence {
                attempt: 1,
                generation: 0,
            },
            lease_token: String::new(),
            records,
            encoded_len: 1_024,
        };
        assert_eq!(
            super::validate(&batch),
            Err(sagat_domain::step_logs::BatchInvalid::TooLarge)
        );
    }

    /// The references a transaction is given print no token hash and no subject.
    #[test]
    fn the_references_of_a_batch_print_no_token_hash_or_subject() {
        let instance = ExecutionInstanceId::root(NodeId::parse("a").expect("a node"));
        let attempt = super::AttemptRef {
            tenant: sagat_domain::identity::TenantId::new(uuid::Uuid::nil()),
            run_id: sagat_domain::execution::RunId::new(uuid::Uuid::nil()),
            step_id: uuid::Uuid::nil(),
            instance: &instance,
            fence: sagat_domain::execution::Fence {
                attempt: 1,
                generation: 0,
            },
            lease_token_hash: &[0xAB; 32],
        };
        let session = super::SessionRef {
            tenant: sagat_domain::identity::TenantId::new(uuid::Uuid::nil()),
            namespace: sagat_domain::identity::NamespaceId::new(uuid::Uuid::nil()),
            worker_id: uuid::Uuid::nil(),
            subject: "worker-subject-name",
            token_hash: &[0xCD; 32],
        };
        let printed = format!("{attempt:?} {session:?}");
        for hidden in ["171", "205", "worker-subject-name"] {
            assert!(!printed.contains(hidden), "{printed}");
        }
    }

    /// The shared counter rises by the stream's saturated rise, split by reason (D041-140 (7)).
    #[test]
    fn the_dropped_counter_rises_by_the_saturated_change_only() {
        let before = StreamCounters {
            dropped_by_server: (1 << 53) - 2,
            ..StreamCounters::new()
        };
        let plan = sagat_domain::step_logs::plan_batch(
            &StreamCounters {
                ceiling_sequence: Some(1),
                ceiling: Some(CeilingReason::AttemptRecords),
                last_sequence: 1,
                ..before
            },
            sagat_domain::step_logs::AttemptCeilings {
                records: 0,
                bytes: 0,
            },
            sagat_domain::step_logs::TenantAllowance { remaining: None },
            &[sagat_domain::step_logs::SizedRecord {
                sequence: 1_000,
                text_bytes: 1,
                truncated: false,
            }],
            None,
        );
        let split = saturated_drops(&before, &plan.after, &plan.drops);
        assert_eq!(split.iter().map(|(_, n)| n).sum::<u64>(), 2);
    }
}
