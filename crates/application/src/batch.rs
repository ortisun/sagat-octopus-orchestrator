//! Batch operations (feature 018, `FR-018-027..034`): a job applies one action to a selection of runs,
//! each in its own transaction at a durable rate. This module is the port of the store, the records it
//! speaks and the service that creates, previews, reads, lists and cancels jobs. Selecting the runs
//! and applying the action to each is the executor's (`BatchExecutor`), which runs in the scheduler.
//!
//! Authority: creating a job needs `run:batch` and every permission of the single action
//! (`FR-018-034`); reading a job needs `run:read`, cancelling one `run:batch`. The tenant comes from
//! the principal and a job of another tenant is as if it did not exist.

use std::future::Future;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::batch::{
    Action, BatchError, DeferCause, FailureReason, Filter, ItemState, JobState, Justification,
    Limits, ParamsGiven, Rate, Selector, SelectorKind, SkipReason,
};
use sagat_domain::execution::RunStatus;
use sagat_domain::identity::{NamespaceId, Permission, Principal, TenantId};
use sagat_domain::quota::QuotaResource;
use sagat_domain::sharding::Shard;
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};
use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use crate::clock::Clock;
use crate::definitions::{Applied, Idempotency, fingerprint};
use crate::error::{AppError, StoreError};
use crate::identity::{Page, PageRequest, RequestContext};
use crate::quotas::QuotaLookup;
use crate::schedules::{ShardFence, StoredInput};

/// How long a finished job and its items are kept (`FR-018-033`): 30 days by default, at most 90.
pub const DEFAULT_RETENTION: Duration = Duration::days(30);
pub const MAX_RETENTION: Duration = Duration::days(90);

/// The most a signal payload or the arguments of an update may weigh, as for the single-run actions.
pub const MAX_PAYLOAD_BYTES: usize = 256 * 1024;

/// How many sample ids a preview returns.
pub const PREVIEW_SAMPLE: usize = 20;

/// The counters of a job: they equal the outcomes of its items (`FR-018-031`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub selected: u32,
    pub succeeded: u32,
    pub skipped: u32,
    pub failed: u32,
    pub cancelled: u32,
    pub deferred: u32,
}

/// A job as it is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRecord {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    pub action: Action,
    /// The parameters of the action, closed per action ([`JobParams::to_json`]).
    pub params: Value,
    /// The signal payload or the update arguments, a payload of the tenant; never returned.
    pub payload_digest: Option<[u8; 32]>,
    pub selector: Selector,
    pub reason: String,
    pub created_by: String,
    pub state: JobState,
    pub failure_reason: Option<FailureReason>,
    pub max_items: u32,
    pub rate: Rate,
    pub counts: Counts,
    /// The keyset cursor of the selection: the last run id looked at.
    pub select_cursor: Option<Uuid>,
    /// The selection looks at the runs created at or before this instant (the database clock).
    pub selection_bound_at: OffsetDateTime,
    /// The earliest instant the next item may be applied.
    pub next_slot_at: OffsetDateTime,
    pub created_at: OffsetDateTime,
    pub started_at: Option<OffsetDateTime>,
    pub finished_at: Option<OffsetDateTime>,
    pub expires_at: OffsetDateTime,
}

/// An item as it is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRecord {
    pub ordinal: u32,
    pub run_id: Uuid,
    pub state: ItemState,
    /// Why it was skipped or deferred: a closed code.
    pub reason: Option<String>,
    /// The `Error.code` of a failure.
    pub error_code: Option<String>,
    pub attempts: u32,
    pub next_attempt_at: Option<OffsetDateTime>,
    /// When the first refusal happened: an item is retried for a bounded time after it.
    pub first_refused_at: Option<OffsetDateTime>,
    /// The run a rerun created.
    pub new_run_id: Option<Uuid>,
    pub updated_at: OffsetDateTime,
}

/// A string that is not a name this build knows: a stored value written by something else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not a name this build knows")]
pub struct UnknownName;

/// The wire strings of an item state.
#[must_use]
pub const fn item_state_name(state: ItemState) -> &'static str {
    match state {
        ItemState::Pending => "PENDING",
        ItemState::Deferred => "DEFERRED",
        ItemState::Succeeded => "SUCCEEDED",
        ItemState::Skipped => "SKIPPED",
        ItemState::Failed => "FAILED",
        ItemState::Cancelled => "CANCELLED",
    }
}

/// # Errors
/// [`UnknownName`] for a string that is not the name of an item state.
pub fn item_state_from(name: &str) -> Result<ItemState, UnknownName> {
    Ok(match name {
        "PENDING" => ItemState::Pending,
        "DEFERRED" => ItemState::Deferred,
        "SUCCEEDED" => ItemState::Succeeded,
        "SKIPPED" => ItemState::Skipped,
        "FAILED" => ItemState::Failed,
        "CANCELLED" => ItemState::Cancelled,
        _ => return Err(UnknownName),
    })
}

/// # Errors
/// [`UnknownName`] for a string that is not the name of a failure reason.
pub fn failure_reason_from(name: &str) -> Result<FailureReason, UnknownName> {
    Ok(match name {
        "AUTHORITY_REVOKED" => FailureReason::AuthorityRevoked,
        "SELECTION_FAILED" => FailureReason::SelectionFailed,
        _ => return Err(UnknownName),
    })
}

/// A named payload of a request: the name of the signal or of the update, and its body as JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedPayload {
    pub name: String,
    pub body: Vec<u8>,
}

/// What a create request carries, already decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchWrite {
    pub selector: Selector,
    pub action: Action,
    pub step_id: Option<String>,
    pub instance_path: Vec<String>,
    pub signal: Option<NamedPayload>,
    pub update: Option<NamedPayload>,
    /// `RERUN` only: start the new runs without the business id of the source.
    pub drop_business_id: bool,
    /// `REPIN` only (feature 019): refused while that feature is not enabled.
    pub repin: bool,
    pub reason: Justification,
    pub max_items: Option<u32>,
    pub rate: Option<Rate>,
}

/// The closed parameters of an action as they are stored beside the job.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobParams {
    pub step_id: Option<String>,
    pub instance_path: Vec<String>,
    /// The name of the signal or of the update.
    pub name: Option<String>,
    pub drop_business_id: bool,
}

impl JobParams {
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut map = Map::new();
        if let Some(step) = &self.step_id {
            map.insert("stepId".to_owned(), json!(step));
        }
        if !self.instance_path.is_empty() {
            map.insert("instancePath".to_owned(), json!(self.instance_path));
        }
        if let Some(name) = &self.name {
            map.insert("name".to_owned(), json!(name));
        }
        if self.drop_business_id {
            map.insert("dropBusinessId".to_owned(), json!(true));
        }
        Value::Object(map)
    }

    /// # Errors
    /// [`UnknownName`] for parameters that are not what [`Self::to_json`] writes.
    pub fn from_json(value: &Value) -> Result<Self, UnknownName> {
        let map = value.as_object().ok_or(UnknownName)?;
        if map.keys().any(|k| {
            !matches!(
                k.as_str(),
                "stepId" | "instancePath" | "name" | "dropBusinessId"
            )
        }) {
            return Err(UnknownName);
        }
        let text = |key: &str| -> Result<Option<String>, UnknownName> {
            map.get(key)
                .map(|v| v.as_str().map(str::to_owned).ok_or(UnknownName))
                .transpose()
        };
        let instance_path = match map.get("instancePath") {
            None => Vec::new(),
            Some(Value::Array(items)) => items
                .iter()
                .map(|v| v.as_str().map(str::to_owned).ok_or(UnknownName))
                .collect::<Result<_, _>>()?,
            Some(_) => return Err(UnknownName),
        };
        Ok(Self {
            step_id: text("stepId")?,
            instance_path,
            name: text("name")?,
            drop_business_id: match map.get("dropBusinessId") {
                None => false,
                Some(Value::Bool(flag)) => *flag,
                Some(_) => return Err(UnknownName),
            },
        })
    }
}

// --- the selector, as the store keeps it -----------------------------------------------------

/// The selector as stored: `{"runIds": [...]}` or `{"filter": {...}}`, the shape of the contract.
#[must_use]
pub fn selector_to_json(selector: &Selector) -> Value {
    match selector.kind() {
        SelectorKind::RunIds(ids) => {
            json!({"runIds": ids.iter().map(Uuid::to_string).collect::<Vec<_>>()})
        }
        SelectorKind::Filter(filter) => {
            let mut map = Map::new();
            if let Some(id) = filter.workflow_id {
                map.insert("workflowId".to_owned(), json!(id.to_string()));
            }
            if let Some(id) = filter.version_id {
                map.insert("versionId".to_owned(), json!(id.to_string()));
            }
            if !filter.statuses.is_empty() {
                map.insert(
                    "status".to_owned(),
                    json!(
                        filter
                            .statuses
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                    ),
                );
            }
            if let Some(origin) = &filter.origin {
                map.insert("origin".to_owned(), json!(origin));
            }
            if let Some(id) = filter.schedule_id {
                map.insert("scheduleId".to_owned(), json!(id.to_string()));
            }
            for (key, at) in [
                ("createdAfter", filter.created_after),
                ("createdBefore", filter.created_before),
            ] {
                if let Some(text) = at.and_then(|at| at.format(&Rfc3339).ok()) {
                    map.insert(key.to_owned(), json!(text));
                }
            }
            if let Some(prefix) = &filter.business_id_prefix {
                map.insert("businessIdPrefix".to_owned(), json!(prefix));
            }
            json!({"filter": Value::Object(map)})
        }
    }
}

/// # Errors
/// [`BatchError::SelectorInvalid`] for a document that is not a selector, or one the domain refuses.
pub fn selector_from_json(value: &Value) -> Result<Selector, BatchError> {
    let invalid = || BatchError::SelectorInvalid;
    let map = value.as_object().ok_or_else(invalid)?;
    if map.len() != 1 {
        return Err(invalid());
    }
    if let Some(ids) = map.get("runIds") {
        let ids = ids
            .as_array()
            .ok_or_else(invalid)?
            .iter()
            .map(|v| {
                v.as_str()
                    .and_then(|s| s.parse::<Uuid>().ok())
                    .ok_or_else(invalid)
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Selector::run_ids(ids);
    }
    let filter = map
        .get("filter")
        .and_then(Value::as_object)
        .ok_or_else(invalid)?;
    let uuid = |key: &str| -> Result<Option<Uuid>, BatchError> {
        filter
            .get(key)
            .map(|v| {
                v.as_str()
                    .and_then(|s| s.parse::<Uuid>().ok())
                    .ok_or_else(invalid)
            })
            .transpose()
    };
    let instant = |key: &str| -> Result<Option<OffsetDateTime>, BatchError> {
        filter
            .get(key)
            .map(|v| {
                v.as_str()
                    .and_then(|s| OffsetDateTime::parse(s, &Rfc3339).ok())
                    .ok_or_else(invalid)
            })
            .transpose()
    };
    let text = |key: &str| -> Result<Option<String>, BatchError> {
        filter
            .get(key)
            .map(|v| v.as_str().map(str::to_owned).ok_or_else(invalid))
            .transpose()
    };
    let statuses = match filter.get("status") {
        None => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| {
                v.as_str()
                    .and_then(|s| s.parse::<RunStatus>().ok())
                    .ok_or_else(invalid)
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err(invalid()),
    };
    Selector::filter(Filter {
        workflow_id: uuid("workflowId")?,
        version_id: uuid("versionId")?,
        statuses,
        origin: text("origin")?,
        schedule_id: uuid("scheduleId")?,
        created_after: instant("createdAfter")?,
        created_before: instant("createdBefore")?,
        business_id_prefix: text("businessIdPrefix")?,
    })
}

// --- the port ---------------------------------------------------------------------------------

/// A job to create, with the payload of its action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewJob {
    /// State `SELECTING`, no counts. The store sets the selection bound, the horizon and the first
    /// slot from the database's clock and transaction snapshot.
    pub record: JobRecord,
    pub payload: Option<StoredInput>,
}

/// The ceilings a new job is judged against: the jobs that have not finished, in the namespace and in
/// the tenant (`FR-018-033`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveJobLimits {
    pub namespace: u64,
    pub tenant: u64,
}

/// What a preview found: how many runs match (counted up to `max_items + 1`) and a few of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub matched: u32,
    /// More than `max_items` matched.
    pub capped: bool,
    pub sample: Vec<Uuid>,
}

/// A job the pass should look at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DueJob {
    pub tenant_id: TenantId,
    pub id: Uuid,
}

/// What the scan of one round found, in one statement and at one instant of the database's clock (the start of the scan).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DueJobs {
    /// The jobs to look at, earliest slot first.
    pub jobs: Vec<DueJob>,
    /// How long after the start of the scan the soonest slot that was still ahead of that instant comes, over the running
    /// jobs of the shards (measured by the database's clock when the scan ends, never negative: a slot that came while the
    /// scan ran is zero). A slot that had already come at the start of the scan is not here: it is a due job, or a job with
    /// nothing to do yet, which the next round would find the same. `None` when no running job has a slot ahead.
    pub next_slot_in: Option<std::time::Duration>,
}

/// What a selection round did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selected {
    /// A transaction that was open when the job was created has not ended: the selection starts
    /// once every one of them has (`FR-018-029`). Or the job waits the backoff that a failed round put on it
    /// (`next_slot_at` is ahead of the database's clock).
    Waiting,
    /// A page of runs became items; the job runs once the selection has reached its end or `maxItems`.
    Picked { items: u32 },
    /// The job is no longer selecting.
    NotSelecting,
}

/// What the record of a failed selection round did (G2093, D2197).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionFailure {
    /// The round is counted and the job waits its backoff (`next_slot_at`, by the database's clock) before the next round.
    /// `failures` is how many rounds have failed in a row, this one included.
    Retry { failures: u32 },
    /// That was one failure too many: the job is `FAILED` with `SELECTION_FAILED`, in the same transaction, by the same
    /// writes as a job that fails for another reason (the items that had not ended `CANCELLED`, the audit record).
    Failed { failures: u32 },
    /// The job is no longer selecting (cancelled, ended): nothing was recorded.
    NotSelecting,
}

/// What the executor applies next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NextItem {
    Item(ItemRecord),
    /// Every selected item has ended.
    Drained,
    /// Items remain, but none is due: the deferred ones wait for their retry.
    Waiting,
}

/// How an item ended, or why it is put off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemOutcome {
    Succeeded {
        /// The run a rerun created.
        new_run: Option<Uuid>,
    },
    Skipped(SkipReason),
    /// A closed `Error.code`.
    Failed {
        code: &'static str,
    },
    /// Refused for now; tried again at `retry_at`.
    Deferred {
        cause: DeferCause,
        retry_at: OffsetDateTime,
    },
}

/// What the transaction of a run does for the batch item that drives it (O1951, ADR-0050 decision 2, `FR-018-030`): the
/// single-run use case hands it to [`crate::execution::RunTx::settle_batch_item`] after its effect and before it commits,
/// so the fence of the shard, the job and the item are locked after the run (slot 3R, then 8a; the lease row is a leaf
/// lock), checked (the lease is the replica's, the job runs, the item has not ended) and the outcome, the counters and
/// the next slot of the job written in the run's own transaction. A refusal there rolls the effect back. Nothing else of
/// the item is decided by the use case: a refusal of the action itself rolls the run's transaction back and the executor
/// records it as before, because nothing happened to the run.
///
/// The use case says which outcome it committed with the effect through [`Self::committed`], once its transaction has
/// committed; the executor reads it back with [`Self::settled`] and records nothing more.
#[derive(Debug)]
pub struct ItemSettlement {
    pub tenant: TenantId,
    pub job: Uuid,
    pub ordinal: u32,
    /// The run of the item: the run the transaction acts on, or the run a rerun repeats. The record refuses any other.
    pub run: Uuid,
    pub fence: Option<ShardFence>,
    /// The instant of the record (the executor's clock, as the record of a refusal has it).
    pub at: OffsetDateTime,
    /// The audit record of the end of the job, written when this item is the last.
    pub finished: AuditRecord,
    settled: std::sync::OnceLock<ItemOutcome>,
}

impl ItemSettlement {
    #[must_use]
    pub const fn new(
        tenant: TenantId,
        job: Uuid,
        ordinal: u32,
        run: Uuid,
        fence: Option<ShardFence>,
        at: OffsetDateTime,
        finished: AuditRecord,
    ) -> Self {
        Self {
            tenant,
            job,
            ordinal,
            run,
            fence,
            at,
            finished,
            settled: std::sync::OnceLock::new(),
        }
    }

    /// Says that `outcome` committed with the effect. Called after the run's transaction committed, never before.
    pub fn committed(&self, outcome: ItemOutcome) {
        // one transaction commits per item: a second call would be a defect of the use case, and the first stands
        let _ = self.settled.set(outcome);
    }

    /// The outcome that committed with the effect, `None` when the use case committed none (a replay of an effect that
    /// committed before this item's record existed, or a refusal).
    #[must_use]
    pub fn settled(&self) -> Option<ItemOutcome> {
        self.settled.get().copied()
    }
}

/// What a drain of a cancelled job did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Drained {
    pub cancelled: u32,
    /// Nothing was left to cancel and the job is `CANCELLED`.
    pub finished: bool,
}

/// Persistence of batch jobs. Implemented by `sagat-infra-postgres`.
pub trait BatchStore: QuotaLookup + Send + Sync {
    fn namespace_exists(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Creates the job, judged against `limits`, with its audit record and the request key, in one
    /// transaction. A request that repeats a recorded one writes nothing and is
    /// [`Applied::Replayed`] with the id of the job it created; the same key with a different request
    /// is [`StoreError::IdempotencyKeyReused`]; a full namespace or tenant is
    /// [`StoreError::QuotaExceeded`].
    fn create_job(
        &self,
        new: &NewJob,
        limits: ActiveJobLimits,
        audit: &AuditRecord,
        idempotency: &Idempotency,
    ) -> impl Future<Output = Result<Applied<()>, StoreError>> + Send;

    fn job(
        &self,
        tenant: TenantId,
        id: Uuid,
    ) -> impl Future<Output = Result<Option<JobRecord>, StoreError>> + Send;

    /// The jobs of a namespace, newest first (ids are time-ordered), those after `after` in that order.
    fn jobs(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        state: Option<JobState>,
        after: Option<Uuid>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<JobRecord>, StoreError>> + Send;

    /// The items of a job in order of ordinal, those after `after`.
    fn items(
        &self,
        tenant: TenantId,
        job: Uuid,
        state: Option<ItemState>,
        after: Option<u32>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<ItemRecord>, StoreError>> + Send;

    /// Asks the job to stop: new items stop after the one in flight, the rest are marked cancelled
    /// by the executor (`FR-018-032`). A job that is already cancelling or over is returned as it is. The job is due at once:
    /// its slot (the next item's, or the end of the wait after a failed selection round) comes back to the database's now.
    fn request_cancel(
        &self,
        tenant: TenantId,
        id: Uuid,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<Option<JobRecord>, StoreError>> + Send;

    /// The last look before an effect is applied to a run: the lease is still the replica's, the job
    /// still runs (not cancelled, not over) and the item has not ended. Anything else is a
    /// `StateConflict`. It spares the run's transaction a wasted effect; what decides is the same check
    /// made again inside that transaction, under the locks of the job and the item
    /// ([`ItemSettlement`]), so a lease lost or a cancel made after this look rolls the effect back.
    fn ensure_live(
        &self,
        tenant: TenantId,
        id: Uuid,
        ordinal: u32,
        fence: Option<&ShardFence>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// The jobs of the shards the replica owns that are due to be looked at, earliest first. A job
    /// that has nothing to do yet is not among them (a selection that waits for a transaction, items
    /// that are all deferred to a later time): it would keep the head of every round.
    ///
    /// **One statement, one instant**: the jobs and the time to the soonest slot still ahead (what the loop sleeps for when
    /// no job is due) are answered by one statement that reads the database's clock once, at its start, so a slot that comes
    /// between the scan and any later look is still ahead of that instant and is never lost (G2043).
    fn due_jobs(
        &self,
        shards: &[Shard],
        limit: u32,
    ) -> impl Future<Output = Result<DueJobs, StoreError>> + Send;

    /// One round of the selection: once every transaction that was open when the job was created
    /// has ended, the next page of the runs the selector matches (those created at or before the
    /// bound of the job, by run id after the cursor) become items, in one transaction with the
    /// cursor and the counters; the count of consecutive failed rounds goes back to zero in that same transaction (a round
    /// that gets as far as picking is progress). A job whose next round is put off by a backoff ([`Self::selection_failed`])
    /// is [`Selected::Waiting`] until the database's clock reaches it. A lost lease is a `StateConflict` and commits nothing.
    fn select_round(
        &self,
        tenant: TenantId,
        id: Uuid,
        fence: Option<&ShardFence>,
        page: u32,
    ) -> impl Future<Output = Result<Selected, StoreError>> + Send;

    /// Records that a selection round of the job failed (the round rolled back its own work), under the fence of the shard,
    /// in one transaction: the count of consecutive failed rounds goes up by one and the job's next round is put off by the
    /// backoff of `sagat_domain::batch::selection_retry` (`next_slot_at`, read from the database's clock), or, when the
    /// count passes the bound, the job is ended as `FailureReason::SelectionFailed` by the writes of
    /// [`fail_job`](Self::fail_job). A job that is not selecting records nothing ([`SelectionFailure::NotSelecting`]); a
    /// lost lease is a `StateConflict` and commits nothing. The error of the round is not an argument: its text can quote a
    /// value of the selector, and nothing of it is kept (the executor reads its SQLSTATE, `StoreError::sqlstate`, for its log).
    ///
    /// **The store decides retry or end, not the executor** (D2197): the count, the wait and the end must be one transaction
    /// under one lock of the job row, and that decision is made on the count read under that lock. An executor that read the
    /// count, decided and then asked the store to write it would decide on a number another step, or a cancel, can have moved
    /// (D2204). The answer is a type with meaning of its own, [`SelectionFailure`]: the job waits, the job ended, or it is not
    /// selecting any more. The policy itself is the domain's (`selection_retry`), which the store applies.
    fn selection_failed(
        &self,
        tenant: TenantId,
        id: Uuid,
        at: OffsetDateTime,
        fence: Option<&ShardFence>,
        finished: &AuditRecord,
    ) -> impl Future<Output = Result<SelectionFailure, StoreError>> + Send;

    /// The next item to apply: the first that is pending, or deferred and due.
    fn next_item(
        &self,
        tenant: TenantId,
        id: Uuid,
        at: OffsetDateTime,
    ) -> impl Future<Output = Result<NextItem, StoreError>> + Send;

    /// Records the outcome of an item with the counters and the next slot of the job, under the
    /// fence of the shard, in one transaction: an item the action did not apply (a skip, a failure, a
    /// refusal for now), or one whose effect committed before this record existed (a replay of its
    /// key). An effect applied now is recorded in the run's transaction instead ([`ItemSettlement`]).
    /// A job that is over or an item that has ended is a `StateConflict`. `conflict`, when given, is the
    /// `business_id.conflict` record of an item the platform refused because its business id is held: it commits
    /// with the item or not at all (D2220).
    #[allow(clippy::too_many_arguments)]
    fn record_item(
        &self,
        tenant: TenantId,
        id: Uuid,
        ordinal: u32,
        outcome: &ItemOutcome,
        at: OffsetDateTime,
        fence: Option<&ShardFence>,
        finished: &AuditRecord,
        conflict: Option<&AuditRecord>,
    ) -> impl Future<Output = Result<JobRecord, StoreError>> + Send;

    /// Marks a job whose items have all ended `COMPLETED`.
    fn complete_job(
        &self,
        tenant: TenantId,
        id: Uuid,
        at: OffsetDateTime,
        fence: Option<&ShardFence>,
        finished: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Marks up to `limit` of the items that have not ended of a cancelling job `CANCELLED`; when none
    /// is left the job is `CANCELLED` (`FR-018-032`).
    fn drain_cancel(
        &self,
        tenant: TenantId,
        id: Uuid,
        limit: u32,
        at: OffsetDateTime,
        fence: Option<&ShardFence>,
        finished: &AuditRecord,
    ) -> impl Future<Output = Result<Drained, StoreError>> + Send;

    /// Ends a job `FAILED` for `reason`; the items that had not ended are marked `CANCELLED`.
    fn fail_job(
        &self,
        tenant: TenantId,
        id: Uuid,
        reason: FailureReason,
        at: OffsetDateTime,
        fence: Option<&ShardFence>,
        finished: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// The bytes of a payload of the tenant (the signal or the update arguments of a job).
    fn payload(
        &self,
        tenant: TenantId,
        digest: &[u8; 32],
    ) -> impl Future<Output = Result<Option<Vec<u8>>, StoreError>> + Send;

    /// Counts the runs a selector matches in a namespace, created at or before `bound`, up to
    /// `max_items + 1`, and samples a few of their ids. Changes nothing, within a bounded time.
    fn preview(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        selector: &Selector,
        max_items: u32,
        sample: usize,
    ) -> impl Future<Output = Result<Preview, StoreError>> + Send;
}

// --- the service ------------------------------------------------------------------------------

fn invalid(reason: &'static str, field: &str) -> AppError {
    AppError::InvalidBatch {
        reason,
        field: Some(field.to_owned()),
    }
}

/// The name of a signal as the single-run route takes it: 1 to 128 characters, starting with a
/// letter or digit, then letters, digits and `_ . : -`.
fn valid_signal_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 128
        && chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'))
}

/// Every permission of the single action, besides `run:batch` (`FR-018-034`).
#[must_use]
pub fn action_permissions(action: Action) -> &'static [Permission] {
    match action {
        Action::Cancel | Action::Pause | Action::Resume => &[Permission::RunOperate],
        Action::RetryStep => &[Permission::RunOperate],
        Action::MarkManualReview => &[Permission::RunOperate],
        Action::Signal => &[Permission::RunSignal],
        Action::Update => &[Permission::RunUpdate],
        Action::Rerun => &[Permission::RunOperate, Permission::RunStart],
        Action::Repin => &[Permission::RunOperate],
    }
}

/// Creates, previews, reads, lists and cancels batch jobs.
pub struct BatchService<S, C> {
    store: S,
    clock: C,
    retention: Duration,
}

impl<S: BatchStore, C: Clock> BatchService<S, C> {
    pub fn new(store: S, clock: C) -> Self {
        Self {
            store,
            clock,
            retention: DEFAULT_RETENTION,
        }
    }

    /// Keeps jobs for `retention` instead of the default, held inside the hard ceiling.
    #[must_use]
    pub fn with_retention(mut self, retention: Duration) -> Self {
        self.retention = retention.clamp(Duration::days(1), MAX_RETENTION);
        self
    }

    pub const fn store(&self) -> &S {
        &self.store
    }

    /// Creates a job and answers it, `SELECTING`; the selection and the items are the executor's.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `run:batch` or a permission of the action;
    /// [`AppError::NotFound`] for a namespace that is not the tenant's; [`AppError::InvalidBatch`]
    /// for parameters the action does not take (`BATCH_ACTION_PARAMETERS`), a feature that is not enabled
    /// (`BATCH_ACTION_UNAVAILABLE`), limits out of range (`BATCH_LIMITS_INVALID`), a payload that is
    /// not JSON or is too big, or a selector the deployment cannot evaluate;
    /// [`AppError::QuotaExceeded`] for a full namespace or tenant;
    /// [`AppError::IdempotencyKeyReused`].
    pub async fn create(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        write: BatchWrite,
        key: &str,
        ctx: &RequestContext,
    ) -> Result<JobRecord, AppError> {
        principal.authorize(Permission::RunBatch)?;
        for needed in action_permissions(write.action) {
            principal.authorize(*needed)?;
        }
        if !(8..=160).contains(&key.chars().count()) {
            return Err(AppError::NotAllowed(
                "the Idempotency-Key must have 8 to 160 characters",
            ));
        }
        if write.action.needs_feature_019() {
            // a job that asked for it would only fail every item
            return Err(invalid("BATCH_ACTION_UNAVAILABLE", "/action"));
        }
        let given = ParamsGiven {
            step_id: write.step_id.is_some(),
            signal: write.signal.is_some(),
            update: write.update.is_some(),
            repin: write.repin,
        };
        if write.action.check_params(given).is_err()
            || (write.drop_business_id && write.action != Action::Rerun)
            || (!write.instance_path.is_empty() && write.step_id.is_none())
        {
            return Err(invalid("BATCH_ACTION_PARAMETERS", "/"));
        }
        if write
            .step_id
            .as_deref()
            .is_some_and(|step| sagat_domain::workflow::NodeId::parse_stored(step).is_err())
        {
            return Err(invalid("BATCH_ACTION_PARAMETERS", "/stepId"));
        }
        if write
            .signal
            .as_ref()
            .is_some_and(|signal| !valid_signal_name(&signal.name))
        {
            return Err(invalid("BATCH_ACTION_PARAMETERS", "/signal/name"));
        }
        // the operation an Update names is one a run can declare: a name that cannot be one would fail
        // every item
        if write.update.as_ref().is_some_and(|update| {
            sagat_domain::workflow::ActivityName::parse(&update.name).is_err()
        }) {
            return Err(invalid("BATCH_ACTION_PARAMETERS", "/update/name"));
        }
        if !write.instance_path.is_empty() {
            // a step of an iteration of a collection or a loop needs its scope path resolved per run;
            // until then a job names root steps only (G1956)
            return Err(invalid("BATCH_ACTION_PARAMETERS", "/instancePath"));
        }
        let limits = Limits::new(
            write.max_items.unwrap_or(Limits::default().max_items),
            write.rate.unwrap_or_default(),
        )
        .map_err(|_| invalid("BATCH_LIMITS_INVALID", "/maxItems"))?;
        if !self
            .store
            .namespace_exists(principal.tenant_id(), namespace)
            .await?
        {
            return Err(AppError::NotFound);
        }
        let (name, body, kind) = match (&write.signal, &write.update) {
            (Some(p), None) => (Some(p), Some(&p.body), "signal"),
            (None, Some(p)) => (Some(p), Some(&p.body), "update"),
            _ => (None, None, ""),
        };
        let payload = match body {
            None => None,
            Some(bytes) => {
                if bytes.len() > MAX_PAYLOAD_BYTES {
                    return Err(invalid(
                        "BATCH_PAYLOAD_TOO_LARGE",
                        if kind == "signal" {
                            "/signal"
                        } else {
                            "/update"
                        },
                    ));
                }
                if serde_json::from_slice::<Value>(bytes).is_err() {
                    return Err(invalid(
                        "BATCH_PAYLOAD_INVALID",
                        if kind == "signal" {
                            "/signal"
                        } else {
                            "/update"
                        },
                    ));
                }
                Some(StoredInput {
                    digest: Sha256::digest(bytes).into(),
                    bytes: bytes.clone(),
                })
            }
        };
        let params = JobParams {
            step_id: write.step_id.clone(),
            instance_path: write.instance_path.clone(),
            name: name.map(|p| p.name.clone()),
            drop_business_id: write.drop_business_id,
        }
        .to_json();
        let tenant = principal.tenant_id();
        let active = ActiveJobLimits {
            namespace: self
                .store
                .effective_limit(tenant, Some(namespace), QuotaResource::BatchJobs)
                .await?,
            tenant: self
                .store
                .effective_limit(tenant, None, QuotaResource::TenantBatchJobs)
                .await?,
        };
        let now = self.clock.now();
        let id = Uuid::now_v7();
        let record = JobRecord {
            id,
            tenant_id: tenant,
            namespace_id: namespace,
            action: write.action,
            params,
            payload_digest: payload.as_ref().map(|p| p.digest),
            selector: write.selector.clone(),
            reason: write.reason.as_str().to_owned(),
            created_by: principal.subject().as_str().to_owned(),
            state: JobState::Selecting,
            failure_reason: None,
            max_items: limits.max_items,
            rate: limits.rate,
            counts: Counts::default(),
            select_cursor: None,
            selection_bound_at: now,
            next_slot_at: now,
            created_at: now,
            started_at: None,
            finished_at: None,
            expires_at: now + self.retention,
        };
        let digest_bytes = payload.as_ref().map_or([0_u8; 32], |p| p.digest);
        let idempotency = Idempotency {
            scope: format!("batch.create:{namespace}"),
            key: key.to_owned(),
            fingerprint: fingerprint(&[
                principal.subject().as_str().as_bytes(),
                write.action.as_str().as_bytes(),
                selector_to_json(&write.selector).to_string().as_bytes(),
                record.params.to_string().as_bytes(),
                &digest_bytes,
                write.reason.as_str().as_bytes(),
                &limits.max_items.to_be_bytes(),
                &limits.rate.milli().to_be_bytes(),
            ]),
        };
        let audit = AuditRecord::by(
            principal,
            AuditAction::BatchCreated,
            ResourceType::BatchJob,
            id.to_string(),
            ctx.trace_id.clone(),
            now,
        )
        .with_reason(write.action.as_str());
        let created = self
            .store
            .create_job(&NewJob { record, payload }, active, &audit, &idempotency)
            .await?;
        let id = match created {
            Applied::Done(()) => id,
            Applied::Replayed(existing) => {
                existing.parse::<Uuid>().map_err(|_| AppError::Internal)?
            }
        };
        self.store.job(tenant, id).await?.ok_or(AppError::Internal)
    }

    /// How many runs a selector would match now, and a few of them. Changes nothing.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `run:batch` and `run:read`; [`AppError::NotFound`] for a
    /// namespace that is not the tenant's; [`AppError::InvalidBatch`] for a limit out of range or a
    /// selector the deployment cannot evaluate.
    pub async fn preview(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        selector: &Selector,
        max_items: Option<u32>,
    ) -> Result<Preview, AppError> {
        principal.authorize(Permission::RunBatch)?;
        principal.authorize(Permission::RunRead)?;
        let limits = Limits::new(
            max_items.unwrap_or(Limits::default().max_items),
            Rate::default(),
        )
        .map_err(|_| invalid("BATCH_LIMITS_INVALID", "/maxItems"))?;
        if !self
            .store
            .namespace_exists(principal.tenant_id(), namespace)
            .await?
        {
            return Err(AppError::NotFound);
        }
        Ok(self
            .store
            .preview(
                principal.tenant_id(),
                namespace,
                selector,
                limits.max_items,
                PREVIEW_SAMPLE,
            )
            .await?)
    }

    /// The namespace of a job of the caller's tenant. It needs no permission: the caller is
    /// authorized for the namespace by the route, which then checks the job is in it.
    ///
    /// # Errors
    /// [`AppError::NotFound`] for a job that is not the tenant's.
    pub async fn namespace_of(
        &self,
        principal: &Principal,
        id: Uuid,
    ) -> Result<NamespaceId, AppError> {
        self.store
            .job(principal.tenant_id(), id)
            .await?
            .map(|job| job.namespace_id)
            .ok_or(AppError::NotFound)
    }

    /// # Errors
    /// [`AppError::Forbidden`] without `run:read`; [`AppError::NotFound`].
    pub async fn get(&self, principal: &Principal, id: Uuid) -> Result<JobRecord, AppError> {
        principal.authorize(Permission::RunRead)?;
        self.store
            .job(principal.tenant_id(), id)
            .await?
            .ok_or(AppError::NotFound)
    }

    /// The jobs of a namespace, newest first.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `run:read`; [`AppError::NotFound`] for a namespace that is not
    /// the tenant's; [`AppError::NotAllowed`] for a cursor that is not one of ours.
    pub async fn list(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        state: Option<JobState>,
        page: &PageRequest,
    ) -> Result<Page<JobRecord>, AppError> {
        principal.authorize(Permission::RunRead)?;
        if !self
            .store
            .namespace_exists(principal.tenant_id(), namespace)
            .await?
        {
            return Err(AppError::NotFound);
        }
        let after = page
            .cursor
            .as_deref()
            .map(|c| {
                c.parse::<Uuid>()
                    .map_err(|_| AppError::NotAllowed("the cursor is not valid"))
            })
            .transpose()?;
        let limit = page.effective_limit();
        let mut rows = self
            .store
            .jobs(principal.tenant_id(), namespace, state, after, limit + 1)
            .await?;
        let next_cursor = if rows.len() > limit as usize {
            rows.truncate(limit as usize);
            rows.last().map(|r| r.id.to_string())
        } else {
            None
        };
        Ok(Page {
            items: rows,
            next_cursor,
        })
    }

    /// The items of a job in order.
    ///
    /// # Errors
    /// As [`Self::get`]; [`AppError::NotAllowed`] for a cursor that is not one of ours.
    pub async fn items(
        &self,
        principal: &Principal,
        id: Uuid,
        state: Option<ItemState>,
        page: &PageRequest,
    ) -> Result<Page<ItemRecord>, AppError> {
        principal.authorize(Permission::RunRead)?;
        self.get(principal, id).await?;
        let after = page
            .cursor
            .as_deref()
            .map(|c| {
                c.parse::<u32>()
                    .map_err(|_| AppError::NotAllowed("the cursor is not valid"))
            })
            .transpose()?;
        let limit = page.effective_limit();
        let mut rows = self
            .store
            .items(principal.tenant_id(), id, state, after, limit + 1)
            .await?;
        let next_cursor = if rows.len() > limit as usize {
            rows.truncate(limit as usize);
            rows.last().map(|r| r.ordinal.to_string())
        } else {
            None
        };
        Ok(Page {
            items: rows,
            next_cursor,
        })
    }

    /// Stops a job after the item in flight; what was applied stays applied (`FR-018-032`).
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `run:batch`; [`AppError::NotFound`];
    /// [`AppError::BatchConflict`] (`BATCH_NOT_CANCELLABLE`) for a job that already ended.
    pub async fn cancel(
        &self,
        principal: &Principal,
        id: Uuid,
        reason: &str,
        ctx: &RequestContext,
    ) -> Result<JobRecord, AppError> {
        principal.authorize(Permission::RunBatch)?;
        let reason =
            Justification::new(reason).map_err(|_| invalid("BATCH_REASON_INVALID", "/reason"))?;
        let current = self
            .store
            .job(principal.tenant_id(), id)
            .await?
            .ok_or(AppError::NotFound)?;
        if current.state.is_terminal() {
            // nothing to stop, and nothing to audit: the job ended before this request
            return Err(AppError::BatchConflict {
                reason: "BATCH_NOT_CANCELLABLE",
            });
        }
        let now = self.clock.now();
        let audit = AuditRecord::by(
            principal,
            AuditAction::BatchCancelled,
            ResourceType::BatchJob,
            id.to_string(),
            ctx.trace_id.clone(),
            now,
        )
        .with_reason(reason.as_str());
        self.store
            .request_cancel(principal.tenant_id(), id, &audit)
            .await?
            .ok_or(AppError::NotFound)
    }
}
