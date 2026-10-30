//! Schedules (feature 018): the ports their use cases stand on.
//!
//! Only the calendar port exists so far. It is the one piece that needs a time-zone database, so
//! the domain keeps the declared spec and an adapter (`sagat-infra-ir`) makes it mean something,
//! the way it does for the cron triggers of feature 004 ([`crate::dispatch::CronSchedule`]).

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::identity::{NamespaceId, Permission, Principal, TenantId};
use sagat_domain::quota::QuotaResource;
use sagat_domain::schedules::{
    ActionKind, Ceilings, Note, Origin, Overlap, Policies, RemainingActions, ScheduleId,
    ScheduleName, ScheduleSpec, ScheduleTarget, actual_time, jitter_offset_ms, jitter_seed,
};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::clock::Clock;
use crate::error::{AppError, StoreError};
use crate::identity::RequestContext;
use crate::quotas::QuotaLookup;
use crate::schedule_codec::{CodecError, policies_from_json, spec_from_json, target_from_json};

/// How many occurrences a single call may be asked for. A caller that needs more asks again from
/// the last one it got.
pub const MAX_OCCURRENCES_PER_CALL: usize = 10_000;

/// How many of the next occurrences are compared with the minimum spacing when a spec is written.
pub const SPACING_WINDOW: usize = 1_000;

/// Why a spec could not be evaluated or was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CalendarFailure {
    #[error("the time zone is not a known IANA name")]
    TimeZone,
    #[error("a cron string is not valid")]
    Cron,
    /// `SCHEDULE_NEVER_FIRES`: nothing falls inside the bounds from now on.
    #[error("the spec has no occurrence inside its bounds")]
    NeverFires,
    /// `SCHEDULE_TOO_FREQUENT`: two of the next 1,000 occurrences are closer than the minimum spacing.
    #[error("two occurrences are closer than the minimum spacing")]
    TooFrequent,
    #[error("too many occurrences asked for in one call")]
    LimitTooLarge,
}

/// The calendar a spec is evaluated against. Implemented by `sagat-infra-ir`, which holds the
/// time-zone database. Occurrences are *nominal* times: jitter is added by the pass, not here.
pub trait ScheduleCalendar: Send + Sync {
    /// Whether the spec means something at `now`: the zone exists, the cron strings parse, at least
    /// one occurrence falls inside the bounds from `now` on, and no two of the next
    /// [`SPACING_WINDOW`] occurrences are closer than `ceilings.min_spacing_ms`.
    ///
    /// # Errors
    /// [`CalendarFailure`] naming what is wrong.
    fn validate(
        &self,
        spec: &ScheduleSpec,
        now: OffsetDateTime,
        ceilings: &Ceilings,
    ) -> Result<(), CalendarFailure>;

    /// The nominal times strictly after `after` (and at or after the spec's `startAt`, before its
    /// `endAt`), in ascending order, without duplicates, the exclusions removed. At most `limit`, and
    /// none after `until` (inclusive) when given. The result is a function of the spec, the time-zone
    /// database version and the arguments only.
    ///
    /// # Errors
    /// [`CalendarFailure::TimeZone`] and [`CalendarFailure::Cron`] for a spec that cannot be
    /// evaluated, [`CalendarFailure::LimitTooLarge`] over [`MAX_OCCURRENCES_PER_CALL`].
    fn occurrences(
        &self,
        spec: &ScheduleSpec,
        after: OffsetDateTime,
        until: Option<OffsetDateTime>,
        limit: usize,
    ) -> Result<Vec<OffsetDateTime>, CalendarFailure>;

    /// The first nominal time strictly after `after`, or `None` when the spec has ended.
    ///
    /// # Errors
    /// As [`ScheduleCalendar::occurrences`].
    fn next_after(
        &self,
        spec: &ScheduleSpec,
        after: OffsetDateTime,
    ) -> Result<Option<OffsetDateTime>, CalendarFailure> {
        Ok(self.occurrences(spec, after, None, 1)?.first().copied())
    }
}

// --- the schedule, as stored --------------------------------------------------------------------

/// Who owns a schedule: a person through the API, or a cron trigger of a published workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleOrigin {
    Api,
    Trigger,
}

impl ScheduleOrigin {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Api => "API",
            Self::Trigger => "TRIGGER",
        }
    }
}

impl std::str::FromStr for ScheduleOrigin {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "API" => Ok(Self::Api),
            "TRIGGER" => Ok(Self::Trigger),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleState {
    Active,
    Paused,
    /// A tombstone: the name is free, the history is kept.
    Deleted,
}

impl ScheduleState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Paused => "PAUSED",
            Self::Deleted => "DELETED",
        }
    }
}

impl std::str::FromStr for ScheduleState {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "ACTIVE" => Ok(Self::Active),
            "PAUSED" => Ok(Self::Paused),
            "DELETED" => Ok(Self::Deleted),
            _ => Err(()),
        }
    }
}

/// A schedule with its head revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleRecord {
    pub id: ScheduleId,
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    pub name: ScheduleName,
    pub origin: ScheduleOrigin,
    /// The cron trigger of a `TRIGGER` schedule.
    pub trigger_id: Option<Uuid>,
    pub state: ScheduleState,
    /// The head revision; a revision never changes.
    pub revision: u32,
    /// Compare-and-swap token of every change of the schedule.
    pub state_version: u64,
    pub spec: ScheduleSpec,
    pub policies: Policies,
    pub target: ScheduleTarget,
    /// The input of the runs: a payload of the tenant, not part of the target.
    pub input_digest: Option<[u8; 32]>,
    pub note: Option<String>,
    pub note_by: Option<String>,
    pub note_at: Option<OffsetDateTime>,
    /// Whose `run:start` every action is checked against.
    pub authorized_by: String,
    pub remaining_actions: Option<u32>,
    /// The next nominal time, before jitter.
    pub next_action_at: Option<OffsetDateTime>,
    /// The instant that nominal time is acted on, jitter included.
    pub next_actual_at: Option<OffsetDateTime>,
    /// When a time that could not start is tried again.
    pub retry_at: Option<OffsetDateTime>,
    /// When the runs that hold back a waiting action are looked at again.
    pub recheck_at: Option<OffsetDateTime>,
    pub tracked_runs: Vec<Uuid>,
    pub buffered: u32,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// The shard lease a pass holds when it acts for a schedule. An HTTP command (trigger-now) holds
/// none: it does not invent a scheduler lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardFence {
    pub owner: String,
    pub lease: crate::shards::Lease,
}

/// Where a schedule goes once an action has started a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleAfter {
    pub next_action_at: Option<OffsetDateTime>,
    pub next_actual_at: Option<OffsetDateTime>,
    /// The new count of actions that may still start a run; `None` for a schedule with no limit.
    pub remaining_actions: Option<u32>,
    /// Rows recorded in the same transaction: the summaries of the times a recovery did not act on.
    pub extra: Vec<RecordedAction>,
}

/// An action row that has no run: a skip, a wait, a drop, a summary or a revoked authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedAction {
    pub nominal_at: OffsetDateTime,
    pub kind: sagat_domain::schedules::ActionKind,
    pub revision: u32,
    pub outcome: sagat_domain::schedules::Outcome,
    /// The closed code that qualifies the outcome (`TRACKING_LIMIT`, `AUTHORITY_REVOKED`).
    pub reason: Option<&'static str>,
    /// How many times a summary stands for, and the last of them.
    pub summary: Option<(u32, OffsetDateTime)>,
}

/// The `Idempotency-Key` of a manual action, recorded in the transaction that makes the action: a
/// request sent again finds it and is answered with what the first one did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionDedupe {
    pub idempotency: crate::definitions::Idempotency,
    /// The identity of the action the key stands for (`manual-{epoch µs}`).
    pub resource: String,
}

/// A decision that starts no run, committed with the new state of the schedule (`FR-018-005`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleDecision {
    pub tenant_id: TenantId,
    pub schedule: ScheduleId,
    /// The state version the decision was made on.
    pub expected_state_version: u64,
    pub fence: Option<ShardFence>,
    pub actions: Vec<RecordedAction>,
    pub after: ScheduleAfter,
    /// When the schedule stops: its authority was revoked, or a run failed and it asks to pause.
    pub pause: Option<AutoPause>,
    pub at: OffsetDateTime,
    /// Set by a manual action (trigger-now).
    pub dedupe: Option<ActionDedupe>,
    /// Set by a step of a backfill: where it got to, committed with the decision.
    pub backfill: Option<BackfillProgress>,
}

/// A pause the platform makes, with the system note that says why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoPause {
    pub note: String,
}

/// Tracked runs that have ended, to forget, and whether their ending pauses the schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reconcile {
    pub tenant_id: TenantId,
    pub schedule: ScheduleId,
    pub expected_state_version: u64,
    pub fence: Option<ShardFence>,
    pub forget: Vec<Uuid>,
    pub pause: Option<AutoPause>,
    pub at: OffsetDateTime,
}

/// A schedule whose time has come, as the due scan sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueSchedule {
    pub tenant_id: TenantId,
    pub id: ScheduleId,
}

/// A waiting action, the oldest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferedAction {
    pub nominal_at: OffsetDateTime,
    pub kind: sagat_domain::schedules::ActionKind,
    pub revision: u32,
}

/// An action of a schedule that starts a run (`FR-018-005`). It rides on the creation of the run, so
/// the fence check, the compare-and-swap on the schedule, the run, the action row and the new state
/// of the schedule commit together or not at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleAdmission {
    pub schedule: ScheduleId,
    /// With the origin of `kind`, the identity of the action: at most one per schedule.
    pub nominal_at: OffsetDateTime,
    pub kind: sagat_domain::schedules::ActionKind,
    /// The revision the action is taken under.
    pub revision: u32,
    /// The state version the decision was made on: any other change in between refuses the
    /// admission, and the pass decides again.
    pub expected_state_version: u64,
    pub fence: Option<ShardFence>,
    pub after: ScheduleAfter,
    /// Set by a manual action (trigger-now): the key it was asked under, and the audit of the
    /// caller's request, committed with the run.
    pub manual: Option<ManualRequest>,
    /// Set by a step of a backfill: where it got to, committed with the run.
    pub backfill: Option<BackfillProgress>,
}

/// What rides with a manual action when it starts a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualRequest {
    pub dedupe: ActionDedupe,
    pub audit: AuditRecord,
}

impl ScheduleAdmission {
    /// The key the run is admitted under, within the scope `schedule:{id}`: `nominal-{epoch ms}` for
    /// a scheduled or backfilled time and `manual-{epoch µs}` for trigger-now, so a manual action
    /// never shares an identity with a scheduled time.
    #[must_use]
    pub fn run_key(&self) -> String {
        match self.kind.origin() {
            sagat_domain::schedules::Origin::Spec => {
                format!(
                    "nominal-{}",
                    self.nominal_at.unix_timestamp_nanos().div_euclid(1_000_000)
                )
            }
            sagat_domain::schedules::Origin::Manual => {
                format!(
                    "manual-{}",
                    self.nominal_at.unix_timestamp_nanos().div_euclid(1_000)
                )
            }
        }
    }
}

/// The payload of a schedule's input: what the store keeps beside the revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredInput {
    pub digest: [u8; 32],
    pub bytes: Vec<u8>,
}

/// A schedule to create, with its first revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSchedule {
    /// Revision 1, state version 1.
    pub record: ScheduleRecord,
    /// The bytes of `record.input_digest`, when the schedule has an input.
    pub input: Option<StoredInput>,
    pub request_id: Option<String>,
}

/// A new revision of a schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revise {
    /// The desired content: `revision` is `expected_revision + 1`, `state` is `Active` or `Paused`,
    /// `authorized_by` is the updater, and the next times are recomputed from the new spec.
    pub record: ScheduleRecord,
    /// The head revision the caller saw. A different head is a conflict, not an overwrite.
    pub expected_revision: u32,
    pub input: Option<StoredInput>,
    pub request_id: Option<String>,
}

/// The note that comes with a pause or a resume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateNote {
    pub text: String,
    pub by: String,
    pub at: OffsetDateTime,
}

/// A pause or a resume: not a revision of the schedule, a change of its state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateChange {
    pub tenant_id: TenantId,
    pub id: ScheduleId,
    /// `Active` or `Paused`.
    pub state: ScheduleState,
    pub note: Option<StateNote>,
    /// The resumer becomes the authority of the schedule; a pause leaves it alone.
    pub authorized_by: Option<String>,
    /// The next times, recomputed by a resume; a pause clears them.
    pub next_action_at: Option<OffsetDateTime>,
    pub next_actual_at: Option<OffsetDateTime>,
    pub updated_at: OffsetDateTime,
}

/// Whether what a schedule would start exists in its namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetCheck {
    Found,
    WorkflowMissing,
    /// The workflow exists but not the pinned version of it.
    VersionMissing,
}

/// Where a backfill is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackfillState {
    Pending,
    Running,
    Completed,
    Cancelled,
}

impl BackfillState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Running => "RUNNING",
            Self::Completed => "COMPLETED",
            Self::Cancelled => "CANCELLED",
        }
    }

    /// Whether it still has times to act on.
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Pending | Self::Running)
    }
}

impl std::str::FromStr for BackfillState {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "PENDING" => Ok(Self::Pending),
            "RUNNING" => Ok(Self::Running),
            "COMPLETED" => Ok(Self::Completed),
            "CANCELLED" => Ok(Self::Cancelled),
            _ => Err(()),
        }
    }
}

/// A request to act on the nominal times of a past range that have no action yet (`FR-018-011`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackfillRequest {
    pub start: OffsetDateTime,
    /// Exclusive, and not after now.
    pub end: OffsetDateTime,
    /// `BUFFER_ALL` when absent.
    pub overlap: Option<Overlap>,
}

/// A backfill, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackfillRecord {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub schedule: ScheduleId,
    pub start: OffsetDateTime,
    pub end: OffsetDateTime,
    /// The last nominal time acted on (or found already acted on): a crash resumes after it.
    pub cursor: Option<OffsetDateTime>,
    pub overlap: Overlap,
    pub state: BackfillState,
    pub requested_by: String,
    /// How many times of each outcome, `ALREADY_ACTED` included.
    pub counters: BTreeMap<String, u64>,
    pub created_at: OffsetDateTime,
    pub finished_at: Option<OffsetDateTime>,
}

/// A backfill to store, with the limits it is admitted under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewBackfill {
    pub record: BackfillRecord,
    pub namespace: NamespaceId,
    /// Backfills that may be active in the namespace.
    pub namespace_limit: u64,
    /// Backfills that may be active on the schedule.
    pub per_schedule_limit: u32,
}

/// A backfill with a time to act on, found by a pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueBackfill {
    pub tenant_id: TenantId,
    pub schedule: ScheduleId,
    pub id: Uuid,
}

/// How a step moved a backfill, committed in the transaction of the step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackfillProgress {
    pub id: Uuid,
    /// The nominal time the step dealt with.
    pub cursor: OffsetDateTime,
    /// The outcome to count (`STARTED`, `BUFFERED`, `ALREADY_ACTED`, ...); none for the step that
    /// finds no time left.
    pub outcome: Option<&'static str>,
    /// There is no time left: the backfill is complete.
    pub finished: bool,
}

/// A decision of a schedule, as read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionView {
    pub nominal_at: OffsetDateTime,
    pub actual_at: Option<OffsetDateTime>,
    pub kind: ActionKind,
    pub origin: Origin,
    pub revision: u32,
    /// A closed vocabulary that only grows: a reader treats an unknown value as another outcome.
    pub outcome: String,
    pub reason: Option<String>,
    pub run_id: Option<Uuid>,
    pub backfill_id: Option<Uuid>,
    /// How many times a `COALESCED` or `MISSED_CATCHUP_WINDOW` summary stands for.
    pub summary_count: Option<u32>,
    pub summary_last_nominal_at: Option<OffsetDateTime>,
    pub decided_at: OffsetDateTime,
}

/// The counters of a schedule and its latest decisions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScheduleActivity {
    pub counters: BTreeMap<String, u64>,
    pub recent: Vec<ActionView>,
}

/// A schedule as `describe` shows it: the record, the times it will act on next and what it has done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleDescription {
    pub record: ScheduleRecord,
    /// `(nominal, actual)` of the next times; empty while the schedule is paused or exhausted.
    pub next: Vec<(OffsetDateTime, OffsetDateTime)>,
    pub activity: ScheduleActivity,
}

/// Storage port of the schedules. Implemented by `sagat-infra-postgres`.
pub trait ScheduleStore: Send + Sync {
    /// Whether the namespace exists in `tenant`.
    fn namespace_exists(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Whether the workflow (and the pinned version, if any) of a target exists in the namespace.
    fn target_check(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        target: &ScheduleTarget,
    ) -> impl Future<Output = Result<TargetCheck, StoreError>> + Send;

    /// Stores a new schedule and its first revision with its audit record, counting it against the
    /// namespace's quota (`limit`, active and paused ones). `Conflict(ScheduleName)` when the name
    /// is taken in the namespace; `QuotaExceeded` when the namespace is full.
    fn create_schedule(
        &self,
        new: &NewSchedule,
        limit: u64,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// The schedule with its head revision. A deleted one is still returned (the caller decides).
    fn schedule(
        &self,
        tenant: TenantId,
        id: ScheduleId,
    ) -> impl Future<Output = Result<Option<ScheduleRecord>, StoreError>> + Send;

    /// The schedules of a namespace that have not been deleted, newest first (ids are time-ordered),
    /// those after `after` in that order when given.
    fn schedules(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        after: Option<ScheduleId>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<ScheduleRecord>, StoreError>> + Send;

    /// Adds a revision and moves the head to it, with its audit record. `None` when the schedule
    /// does not exist for the tenant or was deleted; `RevisionConflict` when the head is not
    /// `expected_revision`; `Conflict(ScheduleName)` when a rename takes a name that is in use.
    fn revise_schedule(
        &self,
        revise: &Revise,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<Option<ScheduleRecord>, StoreError>> + Send;

    /// Pauses or resumes. `None` when the schedule does not exist for the tenant or was deleted.
    fn set_schedule_state(
        &self,
        change: &StateChange,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<Option<ScheduleRecord>, StoreError>> + Send;

    /// The schedules of the given shards whose wake time has come by the database clock, the
    /// earliest first. Only active schedules are due.
    fn due_schedules(
        &self,
        shards: &[sagat_domain::sharding::Shard],
        limit: u32,
    ) -> impl Future<Output = Result<Vec<DueSchedule>, StoreError>> + Send;

    /// The status of each of the given runs of a tenant (a run that is gone is absent).
    fn run_statuses(
        &self,
        tenant: TenantId,
        runs: &[Uuid],
    ) -> impl Future<Output = Result<Vec<(Uuid, sagat_domain::execution::RunStatus)>, StoreError>> + Send;

    /// The oldest action of the schedule that is still waiting.
    fn oldest_buffered(
        &self,
        tenant: TenantId,
        id: ScheduleId,
    ) -> impl Future<Output = Result<Option<BufferedAction>, StoreError>> + Send;

    /// The bytes of a payload of the tenant (the input of a schedule), by digest.
    fn input(
        &self,
        tenant: TenantId,
        digest: &[u8; 32],
    ) -> impl Future<Output = Result<Option<Vec<u8>>, StoreError>> + Send;

    /// The slug of a namespace.
    fn namespace_slug(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> impl Future<Output = Result<Option<String>, StoreError>> + Send;

    /// Records a decision that starts no run, with the new state of the schedule and, when given,
    /// an audit record, in one transaction under the fence and the compare-and-swap. A change in
    /// between, or a lost lease, is a `StateConflict` and commits nothing.
    fn record_decision(
        &self,
        decision: &ScheduleDecision,
        audit: Option<&AuditRecord>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Forgets tracked runs that ended and pauses the schedule when asked, under the same fence.
    fn reconcile_schedule(
        &self,
        reconcile: &Reconcile,
        audit: Option<&AuditRecord>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Asks to look at the schedule again at `at` (a waiting action is held back by an open run),
    /// or clears the request with `None`.
    fn recheck_schedule(
        &self,
        tenant: TenantId,
        id: ScheduleId,
        expected_state_version: u64,
        at: Option<OffsetDateTime>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Keeps a time owed after an admission refusal: the schedule tries it again at `retry_at`,
    /// under the same key, with the time and its state unchanged.
    fn defer_schedule(
        &self,
        tenant: TenantId,
        id: ScheduleId,
        expected_state_version: u64,
        retry_at: OffsetDateTime,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Deletes: the schedule stays as a tombstone, its name is free, its waiting actions become
    /// `DROPPED_DELETED`, its runs are untouched. `false` when there was nothing to delete.
    /// `RevisionConflict` when `expected_revision` is given and the head is another one.
    fn delete_schedule(
        &self,
        tenant: TenantId,
        id: ScheduleId,
        expected_revision: Option<u32>,
        now: OffsetDateTime,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// The action a manual request (trigger-now) made, if the key was used before. A request that
    /// differs from the first one is `IdempotencyKeyReused`.
    fn trigger_replay(
        &self,
        tenant: TenantId,
        idempotency: &crate::definitions::Idempotency,
    ) -> impl Future<Output = Result<Option<ActionView>, StoreError>> + Send;

    /// Stores a backfill with its audit record. `MissingReference` when the schedule is not there or
    /// was deleted; `QuotaExceeded` when the schedule or the namespace has its backfills running.
    fn create_backfill(
        &self,
        new: &NewBackfill,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Whether the schedule already has an action for the nominal time of that origin.
    fn action_exists(
        &self,
        tenant: TenantId,
        schedule: ScheduleId,
        origin: Origin,
        nominal_at: OffsetDateTime,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// The backfills of a schedule, newest first (at most 100).
    fn backfills(
        &self,
        tenant: TenantId,
        schedule: ScheduleId,
    ) -> impl Future<Output = Result<Vec<BackfillRecord>, StoreError>> + Send;

    /// One backfill of a schedule.
    fn backfill(
        &self,
        tenant: TenantId,
        schedule: ScheduleId,
        id: Uuid,
    ) -> impl Future<Output = Result<Option<BackfillRecord>, StoreError>> + Send;

    /// Stops a backfill that is still active (the audit record is written only then); one that has
    /// ended is returned as it is. `None` when there is no such backfill.
    fn cancel_backfill(
        &self,
        tenant: TenantId,
        schedule: ScheduleId,
        id: Uuid,
        now: OffsetDateTime,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<Option<BackfillRecord>, StoreError>> + Send;

    /// The active backfills of schedules in the given shards, the oldest first.
    fn due_backfills(
        &self,
        shards: &[sagat_domain::sharding::Shard],
        limit: u32,
    ) -> impl Future<Output = Result<Vec<DueBackfill>, StoreError>> + Send;

    /// What a schedule has done: its counters and its `recent` latest decisions, newest first.
    fn activity(
        &self,
        tenant: TenantId,
        id: ScheduleId,
        recent: u32,
    ) -> impl Future<Output = Result<ScheduleActivity, StoreError>> + Send;
}

// --- the service --------------------------------------------------------------------------------

/// Largest input of a schedule by default; 256 KiB is the hard ceiling of a run input.
pub const MAX_INPUT_BYTES: usize = 64 * 1024;

/// What a caller writes to create or replace a schedule.
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleWrite {
    pub name: ScheduleName,
    pub spec: ScheduleSpec,
    pub policies: Policies,
    pub target: ScheduleTarget,
    /// The input of every run, an object. Stored beside the revision as a payload.
    pub input: Value,
    /// Create or leave the schedule paused: it acts on no time until it is resumed.
    pub paused: bool,
    pub note: Option<Note>,
    pub remaining_actions: Option<RemainingActions>,
}

/// The first nominal time after `after` and the instant it is acted on (jitter included). The
/// jitter is the digest of the schedule and the nominal time, bounded by the gap to the next nominal
/// time, so every engine process computes the same instant.
///
/// # Errors
/// [`CalendarFailure`] when the spec cannot be evaluated.
pub fn next_times<K: ScheduleCalendar>(
    calendar: &K,
    id: ScheduleId,
    spec: &ScheduleSpec,
    after: OffsetDateTime,
) -> Result<(Option<OffsetDateTime>, Option<OffsetDateTime>), CalendarFailure> {
    let Some(nominal) = calendar.next_after(spec, after)? else {
        return Ok((None, None));
    };
    let gap_ms = calendar
        .next_after(spec, nominal)?
        .and_then(|following| u64::try_from((following - nominal).whole_milliseconds()).ok());
    Ok((Some(nominal), actual_instant(id, spec, nominal, gap_ms)))
}

/// The instant `nominal` is acted on: the jitter of the schedule and the time, bounded by the gap to
/// the next nominal time.
fn actual_instant(
    id: ScheduleId,
    spec: &ScheduleSpec,
    nominal: OffsetDateTime,
    gap_ms: Option<u64>,
) -> Option<OffsetDateTime> {
    let digest: [u8; 32] = Sha256::digest(jitter_seed(id.as_uuid(), nominal)).into();
    let offset = jitter_offset_ms(&digest, spec.jitter_ms(), gap_ms);
    actual_time(nominal, offset, spec.end_at())
}

/// The next `count` times after `after` as `(nominal, actual)`. A time whose jitter would pass the
/// end of the spec is not acted on and is left out.
///
/// # Errors
/// [`CalendarFailure`] when the spec cannot be evaluated.
pub fn upcoming<K: ScheduleCalendar>(
    calendar: &K,
    id: ScheduleId,
    spec: &ScheduleSpec,
    after: OffsetDateTime,
    count: usize,
) -> Result<Vec<(OffsetDateTime, OffsetDateTime)>, CalendarFailure> {
    if count == 0 {
        return Ok(Vec::new());
    }
    let nominals = calendar.occurrences(spec, after, None, count + 1)?;
    Ok(nominals
        .iter()
        .take(count)
        .enumerate()
        .filter_map(|(i, &nominal)| {
            let gap_ms = nominals.get(i + 1).and_then(|following| {
                u64::try_from((*following - nominal).whole_milliseconds()).ok()
            });
            actual_instant(id, spec, nominal, gap_ms).map(|actual| (nominal, actual))
        })
        .collect())
}

fn invalid(reason: &'static str, field: &str) -> AppError {
    AppError::InvalidSchedule {
        reason,
        field: Some(field.to_owned()),
    }
}

fn calendar_error(error: CalendarFailure) -> AppError {
    match error {
        CalendarFailure::TimeZone => invalid("SCHEDULE_ZONE_UNKNOWN", "/spec/timezone"),
        CalendarFailure::Cron => invalid("SCHEDULE_SPEC_INVALID", "/spec/cron"),
        CalendarFailure::NeverFires => invalid("SCHEDULE_NEVER_FIRES", "/spec"),
        CalendarFailure::TooFrequent => invalid("SCHEDULE_TOO_FREQUENT", "/spec"),
        CalendarFailure::LimitTooLarge => {
            AppError::NotAllowed("too many occurrences were asked for at once")
        }
    }
}

/// The input as the payload that is stored: an object of at most [`MAX_INPUT_BYTES`].
fn stored_input(input: &Value) -> Result<StoredInput, AppError> {
    if !input.is_object() {
        return Err(invalid("SCHEDULE_INPUT_INVALID", "/action/input"));
    }
    let bytes = serde_json::to_vec(input)
        .map_err(|_| invalid("SCHEDULE_INPUT_INVALID", "/action/input"))?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(AppError::PayloadTooLarge(
            "the input of a schedule is at most 64 KiB",
        ));
    }
    Ok(StoredInput {
        digest: Sha256::digest(&bytes).into(),
        bytes,
    })
}

/// What a caller sends to create or replace a schedule, before it is checked against the ceilings.
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleDocument {
    pub name: String,
    pub spec: Value,
    /// Absent: every policy takes its default.
    pub policies: Option<Value>,
    /// The target (`workflowId`, ...) with the `input` of the runs.
    pub action: Value,
    pub paused: bool,
    pub note: Option<String>,
    pub remaining_actions: Option<u32>,
}

/// The JSON pointer of a dotted codec path: `spec.calendars[0].hour` is `/spec/calendars/0/hour`.
fn pointer(path: &str) -> String {
    let mut out = String::from("/");
    for c in path.chars() {
        match c {
            '.' | '[' => out.push('/'),
            ']' => {}
            other => out.push(other),
        }
    }
    out
}

fn codec_error(error: &CodecError) -> AppError {
    invalid("SCHEDULE_SPEC_INVALID", &pointer(&error.path))
}

/// Decodes a document against `ceilings`.
///
/// # Errors
/// [`AppError::InvalidSchedule`] naming the field as a JSON pointer.
pub fn decode_document(
    document: ScheduleDocument,
    ceilings: &Ceilings,
) -> Result<ScheduleWrite, AppError> {
    let name =
        ScheduleName::new(&document.name).map_err(|_| invalid("SCHEDULE_SPEC_INVALID", "/name"))?;
    let spec = spec_from_json(&document.spec, ceilings).map_err(|e| codec_error(&e))?;
    let policies = policies_from_json(&document.policies.unwrap_or_else(|| json!({})), ceilings)
        .map_err(|e| codec_error(&e))?;
    let Value::Object(mut action) = document.action else {
        return Err(invalid("SCHEDULE_SPEC_INVALID", "/action"));
    };
    let input = action
        .remove("input")
        .ok_or_else(|| invalid("SCHEDULE_INPUT_INVALID", "/action/input"))?;
    let target = target_from_json(&Value::Object(action)).map_err(|e| codec_error(&e))?;
    let note = document
        .note
        .as_deref()
        .map(Note::new)
        .transpose()
        .map_err(|_| invalid("SCHEDULE_SPEC_INVALID", "/note"))?;
    let remaining_actions = document
        .remaining_actions
        .map(RemainingActions::new)
        .transpose()
        .map_err(|_| invalid("SCHEDULE_SPEC_INVALID", "/remainingActions"))?;
    Ok(ScheduleWrite {
        name,
        spec,
        policies,
        target,
        input,
        paused: document.paused,
        note,
        remaining_actions,
    })
}

/// Administers schedules: who may do what, what a write is checked against, and what the next time
/// of a schedule is. The pass that acts on those times is the scheduler's.
pub struct ScheduleService<S, K, C> {
    store: S,
    calendar: K,
    clock: C,
    ceilings: Ceilings,
}

impl<S, K, C> ScheduleService<S, K, C>
where
    S: ScheduleStore + QuotaLookup,
    K: ScheduleCalendar,
    C: Clock,
{
    #[must_use]
    pub fn new(store: S, calendar: K, clock: C, ceilings: Ceilings) -> Self {
        Self {
            store,
            calendar,
            clock,
            ceilings: ceilings.clamped(),
        }
    }

    /// The ceilings a document is decoded against.
    #[must_use]
    pub const fn ceilings(&self) -> &Ceilings {
        &self.ceilings
    }

    fn next_times(
        &self,
        id: ScheduleId,
        spec: &ScheduleSpec,
        after: OffsetDateTime,
    ) -> Result<(Option<OffsetDateTime>, Option<OffsetDateTime>), AppError> {
        next_times(&self.calendar, id, spec, after).map_err(calendar_error)
    }

    /// Everything a write is checked against before anything is stored.
    async fn check(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        write: &ScheduleWrite,
        now: OffsetDateTime,
    ) -> Result<StoredInput, AppError> {
        if !self
            .store
            .namespace_exists(principal.tenant_id(), namespace)
            .await?
        {
            return Err(AppError::NotFound);
        }
        match self
            .store
            .target_check(principal.tenant_id(), namespace, &write.target)
            .await?
        {
            TargetCheck::Found => {}
            TargetCheck::WorkflowMissing => {
                return Err(invalid("SCHEDULE_SPEC_INVALID", "/action/workflowId"));
            }
            TargetCheck::VersionMissing => {
                return Err(invalid("SCHEDULE_SPEC_INVALID", "/action/versionId"));
            }
        }
        if write.target.business_id().is_some() {
            // PENDING (US03): business identifiers are not enforced yet, so a schedule that asks
            // for one is refused instead of silently starting runs without it.
            return Err(AppError::NotAllowed(
                "business identifiers on schedules are not available yet",
            ));
        }
        self.calendar
            .validate(&write.spec, now, &self.ceilings)
            .map_err(calendar_error)?;
        stored_input(&write.input)
    }

    fn audit(
        principal: &Principal,
        action: AuditAction,
        id: ScheduleId,
        reason: &str,
        ctx: &RequestContext,
        now: OffsetDateTime,
    ) -> AuditRecord {
        AuditRecord::by(
            principal,
            action,
            ResourceType::Schedule,
            id.to_string(),
            ctx.trace_id.clone(),
            now,
        )
        .with_reason(reason)
    }

    /// Creates a schedule. The caller becomes the subject whose `run:start` every action is checked
    /// against.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `schedule:write` and `run:start`; [`AppError::NotFound`] for a
    /// namespace that is not the tenant's; [`AppError::InvalidSchedule`] for a workflow that is not
    /// there, a spec the calendar refuses or an input that is not an object;
    /// [`AppError::PayloadTooLarge`]; [`AppError::Conflict`] for a name in use;
    /// [`AppError::QuotaExceeded`] for a full namespace.
    pub async fn create(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        write: ScheduleWrite,
        ctx: &RequestContext,
    ) -> Result<ScheduleRecord, AppError> {
        principal.authorize(Permission::ScheduleWrite)?;
        principal.authorize(Permission::RunStart)?;
        let now = self.clock.now();
        let input = self.check(principal, namespace, &write, now).await?;
        let id = ScheduleId::generate();
        let state = if write.paused {
            ScheduleState::Paused
        } else {
            ScheduleState::Active
        };
        let (next_action_at, next_actual_at) = if write.paused {
            (None, None)
        } else {
            self.next_times(id, &write.spec, now)?
        };
        let subject = principal.subject().as_str().to_owned();
        let record = ScheduleRecord {
            id,
            tenant_id: principal.tenant_id(),
            namespace_id: namespace,
            name: write.name,
            origin: ScheduleOrigin::Api,
            trigger_id: None,
            state,
            revision: 1,
            state_version: 1,
            spec: write.spec,
            policies: write.policies,
            target: write.target,
            input_digest: Some(input.digest),
            note_by: write.note.as_ref().map(|_| subject.clone()),
            note_at: write.note.as_ref().map(|_| now),
            note: write.note.map(|n| n.as_str().to_owned()),
            authorized_by: subject,
            remaining_actions: write.remaining_actions.map(RemainingActions::get),
            next_action_at,
            next_actual_at,
            retry_at: None,
            recheck_at: None,
            tracked_runs: Vec::new(),
            buffered: 0,
            created_at: now,
            updated_at: now,
        };
        let limit = self
            .store
            .effective_limit(
                principal.tenant_id(),
                Some(namespace),
                QuotaResource::Schedules,
            )
            .await?;
        let audit = Self::audit(
            principal,
            AuditAction::ScheduleCreated,
            id,
            "schedule created",
            ctx,
            now,
        );
        self.store
            .create_schedule(
                &NewSchedule {
                    record: record.clone(),
                    input: Some(input),
                    request_id: None,
                },
                limit,
                &audit,
            )
            .await?;
        Ok(record)
    }

    /// Reads a schedule. A deleted one, another tenant's and one that never existed are all
    /// [`AppError::NotFound`].
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `schedule:read`; [`AppError::NotFound`].
    pub async fn get(
        &self,
        principal: &Principal,
        id: ScheduleId,
    ) -> Result<ScheduleRecord, AppError> {
        principal.authorize(Permission::ScheduleRead)?;
        self.load(principal, id).await
    }

    /// A schedule with the next `next` times it will act on and its `recent` latest decisions.
    ///
    /// # Errors
    /// As [`Self::get`]; [`AppError::InvalidSchedule`] when the stored spec can no longer be evaluated.
    pub async fn describe(
        &self,
        principal: &Principal,
        id: ScheduleId,
        next: u32,
        recent: u32,
    ) -> Result<ScheduleDescription, AppError> {
        principal.authorize(Permission::ScheduleRead)?;
        let record = self.load(principal, id).await?;
        let now = self.clock.now();
        let next = if record.state == ScheduleState::Active {
            upcoming(
                &self.calendar,
                record.id,
                &record.spec,
                now,
                usize::try_from(next.min(100)).unwrap_or(0),
            )
            .map_err(calendar_error)?
        } else {
            Vec::new()
        };
        let activity = self
            .store
            .activity(principal.tenant_id(), id, recent.min(100))
            .await?;
        Ok(ScheduleDescription {
            record,
            next,
            activity,
        })
    }

    /// The namespace a schedule is in. The route has already admitted the caller to a namespace;
    /// this is what it compares with, so a schedule of another namespace is as absent as one of
    /// another tenant.
    ///
    /// # Errors
    /// [`AppError::NotFound`].
    pub async fn namespace_of(
        &self,
        principal: &Principal,
        id: ScheduleId,
    ) -> Result<NamespaceId, AppError> {
        Ok(self.load(principal, id).await?.namespace_id)
    }

    async fn load(
        &self,
        principal: &Principal,
        id: ScheduleId,
    ) -> Result<ScheduleRecord, AppError> {
        match self.store.schedule(principal.tenant_id(), id).await? {
            Some(record) if record.state != ScheduleState::Deleted => Ok(record),
            Some(_) | None => Err(AppError::NotFound),
        }
    }

    /// The schedules of a namespace, newest first, those after `after` in that order.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `schedule:read`; [`AppError::NotFound`] for a namespace that
    /// is not the tenant's.
    pub async fn list(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        after: Option<ScheduleId>,
        limit: u32,
    ) -> Result<Vec<ScheduleRecord>, AppError> {
        principal.authorize(Permission::ScheduleRead)?;
        if !self
            .store
            .namespace_exists(principal.tenant_id(), namespace)
            .await?
        {
            return Err(AppError::NotFound);
        }
        Ok(self
            .store
            .schedules(principal.tenant_id(), namespace, after, limit.clamp(1, 200))
            .await?)
    }

    /// Replaces a schedule with a new revision. The next time is the first one after the update; the
    /// updater becomes the authority of the schedule.
    ///
    /// # Errors
    /// As [`Self::create`], plus [`AppError::RevisionConflict`] when the head is not
    /// `expected_revision` and [`AppError::ScheduleConflict`] for a schedule a cron trigger owns.
    pub async fn update(
        &self,
        principal: &Principal,
        id: ScheduleId,
        write: ScheduleWrite,
        expected_revision: u32,
        ctx: &RequestContext,
    ) -> Result<ScheduleRecord, AppError> {
        principal.authorize(Permission::ScheduleWrite)?;
        principal.authorize(Permission::RunStart)?;
        let current = self.load(principal, id).await?;
        if current.origin == ScheduleOrigin::Trigger {
            return Err(AppError::ScheduleConflict {
                reason: "SCHEDULE_TRIGGER_OWNED",
            });
        }
        if current.revision != expected_revision {
            return Err(AppError::RevisionConflict {
                current: u64::from(current.revision),
            });
        }
        let now = self.clock.now();
        let input = self
            .check(principal, current.namespace_id, &write, now)
            .await?;
        let state = if write.paused {
            ScheduleState::Paused
        } else {
            ScheduleState::Active
        };
        let (next_action_at, next_actual_at) = if write.paused {
            (None, None)
        } else {
            self.next_times(id, &write.spec, now)?
        };
        let mut desired = current;
        desired.name = write.name;
        desired.revision = expected_revision.saturating_add(1);
        desired.state = state;
        desired.spec = write.spec;
        desired.policies = write.policies;
        desired.target = write.target;
        desired.input_digest = Some(input.digest);
        desired.authorized_by = principal.subject().as_str().to_owned();
        desired.remaining_actions = write.remaining_actions.map(RemainingActions::get);
        desired.next_action_at = next_action_at;
        desired.next_actual_at = next_actual_at;
        desired.updated_at = now;
        let audit = Self::audit(
            principal,
            AuditAction::ScheduleUpdated,
            id,
            "schedule updated",
            ctx,
            now,
        );
        self.store
            .revise_schedule(
                &Revise {
                    record: desired,
                    expected_revision,
                    input: Some(input),
                    request_id: None,
                },
                &audit,
            )
            .await?
            .ok_or(AppError::NotFound)
    }

    /// Deletes a schedule: a tombstone that frees the name; its runs are left alone.
    ///
    /// # Errors
    /// [`AppError::Forbidden`]; [`AppError::NotFound`]; [`AppError::ScheduleConflict`] for a schedule
    /// a cron trigger owns.
    pub async fn delete(
        &self,
        principal: &Principal,
        id: ScheduleId,
        expected_revision: Option<u32>,
        ctx: &RequestContext,
    ) -> Result<(), AppError> {
        principal.authorize(Permission::ScheduleWrite)?;
        principal.authorize(Permission::RunStart)?;
        let current = self.load(principal, id).await?;
        if current.origin == ScheduleOrigin::Trigger {
            return Err(AppError::ScheduleConflict {
                reason: "SCHEDULE_TRIGGER_OWNED",
            });
        }
        let now = self.clock.now();
        let audit = Self::audit(
            principal,
            AuditAction::ScheduleDeleted,
            id,
            "schedule deleted",
            ctx,
            now,
        );
        if self
            .store
            .delete_schedule(principal.tenant_id(), id, expected_revision, now, &audit)
            .await?
        {
            Ok(())
        } else {
            Err(AppError::NotFound)
        }
    }

    /// Pauses a schedule with a note. No time is acted on while it is paused, and none is caught up
    /// afterwards.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `schedule:operate`; [`AppError::NotFound`];
    /// [`AppError::InvalidSchedule`] for an empty note; [`AppError::ScheduleConflict`] for a schedule
    /// a cron trigger owns before the trigger handover.
    pub async fn pause(
        &self,
        principal: &Principal,
        id: ScheduleId,
        note: Note,
        ctx: &RequestContext,
    ) -> Result<ScheduleRecord, AppError> {
        principal.authorize(Permission::ScheduleOperate)?;
        self.set_state(principal, id, ScheduleState::Paused, note, ctx)
            .await
    }

    /// Resumes a schedule. The caller becomes its authority, and it continues from the first time
    /// after the resume.
    ///
    /// # Errors
    /// As [`Self::pause`], and [`AppError::Forbidden`] without `run:start`.
    pub async fn resume(
        &self,
        principal: &Principal,
        id: ScheduleId,
        note: Note,
        ctx: &RequestContext,
    ) -> Result<ScheduleRecord, AppError> {
        principal.authorize(Permission::ScheduleOperate)?;
        principal.authorize(Permission::RunStart)?;
        self.set_state(principal, id, ScheduleState::Active, note, ctx)
            .await
    }

    async fn set_state(
        &self,
        principal: &Principal,
        id: ScheduleId,
        state: ScheduleState,
        note: Note,
        ctx: &RequestContext,
    ) -> Result<ScheduleRecord, AppError> {
        if note.as_str().is_empty() {
            return Err(invalid("SCHEDULE_SPEC_INVALID", "/note"));
        }
        let current = self.load(principal, id).await?;
        if current.origin == ScheduleOrigin::Trigger {
            // trigger-owned schedules answer to the legacy cron pass until the operator hands over
            return Err(AppError::ScheduleConflict {
                reason: "SCHEDULE_TRIGGER_HANDOVER_PENDING",
            });
        }
        let now = self.clock.now();
        let resuming = state == ScheduleState::Active;
        let (next_action_at, next_actual_at) = if resuming {
            self.next_times(id, &current.spec, now)?
        } else {
            (None, None)
        };
        let subject = principal.subject().as_str().to_owned();
        let (action, text) = if resuming {
            (AuditAction::ScheduleResumed, "schedule resumed")
        } else {
            (AuditAction::SchedulePaused, "schedule paused")
        };
        let audit = Self::audit(principal, action, id, text, ctx, now);
        self.store
            .set_schedule_state(
                &StateChange {
                    tenant_id: principal.tenant_id(),
                    id,
                    state,
                    note: Some(StateNote {
                        text: note.as_str().to_owned(),
                        by: subject.clone(),
                        at: now,
                    }),
                    authorized_by: resuming.then_some(subject),
                    next_action_at,
                    next_actual_at,
                    updated_at: now,
                },
                &audit,
            )
            .await?
            .ok_or(AppError::NotFound)
    }

    /// Asks to act on the nominal times of a past range that have no action yet, one at a time and
    /// under an overlap rule of its own (`BUFFER_ALL` by default). The caller is whose authority the
    /// runs are started under. A schedule runs one backfill at a time and a namespace the number its
    /// `schedule_backfills` quota says.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `schedule:operate` and `run:start`; [`AppError::NotFound`];
    /// [`AppError::ScheduleConflict`] for a schedule a cron trigger owns;
    /// [`AppError::InvalidSchedule`] with `BACKFILL_RANGE_INVALID` or `BACKFILL_LIMIT`;
    /// [`AppError::QuotaExceeded`].
    pub async fn create_backfill(
        &self,
        principal: &Principal,
        id: ScheduleId,
        request: &BackfillRequest,
        ctx: &RequestContext,
    ) -> Result<BackfillRecord, AppError> {
        principal.authorize(Permission::ScheduleOperate)?;
        principal.authorize(Permission::RunStart)?;
        let schedule = self.load(principal, id).await?;
        if schedule.origin == ScheduleOrigin::Trigger {
            return Err(AppError::ScheduleConflict {
                reason: "SCHEDULE_TRIGGER_HANDOVER_PENDING",
            });
        }
        let now = self.clock.now();
        if request.start >= request.end || request.end > now {
            return Err(invalid("BACKFILL_RANGE_INVALID", "/end"));
        }
        // the range holds its start and not its end
        let times = self
            .calendar
            .occurrences(
                &schedule.spec,
                request.start - time::Duration::milliseconds(1),
                Some(request.end - time::Duration::milliseconds(1)),
                MAX_BACKFILL_ACTIONS + 1,
            )
            .map_err(calendar_error)?;
        if times.len() > MAX_BACKFILL_ACTIONS {
            return Err(invalid("BACKFILL_LIMIT", "/start"));
        }
        let namespace_limit = self
            .store
            .effective_limit(
                principal.tenant_id(),
                Some(schedule.namespace_id),
                QuotaResource::ScheduleBackfills,
            )
            .await?;
        let record = BackfillRecord {
            id: Uuid::now_v7(),
            tenant_id: principal.tenant_id(),
            schedule: id,
            start: request.start,
            end: request.end,
            cursor: None,
            overlap: request.overlap.unwrap_or(Overlap::BufferAll),
            state: BackfillState::Pending,
            requested_by: principal.subject().as_str().to_owned(),
            counters: BTreeMap::new(),
            created_at: now,
            finished_at: None,
        };
        let audit = Self::audit(
            principal,
            AuditAction::ScheduleBackfillRequested,
            id,
            "backfill requested",
            ctx,
            now,
        );
        self.store
            .create_backfill(
                &NewBackfill {
                    record: record.clone(),
                    namespace: schedule.namespace_id,
                    namespace_limit,
                    per_schedule_limit: 1,
                },
                &audit,
            )
            .await?;
        Ok(record)
    }

    /// The backfills of a schedule, newest first.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `schedule:read`; [`AppError::NotFound`].
    pub async fn backfills(
        &self,
        principal: &Principal,
        id: ScheduleId,
    ) -> Result<Vec<BackfillRecord>, AppError> {
        principal.authorize(Permission::ScheduleRead)?;
        self.load(principal, id).await?;
        Ok(self.store.backfills(principal.tenant_id(), id).await?)
    }

    /// Stops a backfill: what it acted on stays acted on. Stopping one that has ended changes
    /// nothing.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `schedule:operate`; [`AppError::NotFound`].
    pub async fn cancel_backfill(
        &self,
        principal: &Principal,
        id: ScheduleId,
        backfill: Uuid,
        ctx: &RequestContext,
    ) -> Result<BackfillRecord, AppError> {
        principal.authorize(Permission::ScheduleOperate)?;
        self.load(principal, id).await?;
        let now = self.clock.now();
        let audit = Self::audit(
            principal,
            AuditAction::ScheduleBackfillCancelled,
            id,
            "backfill cancelled",
            ctx,
            now,
        );
        self.store
            .cancel_backfill(principal.tenant_id(), id, backfill, now, &audit)
            .await?
            .ok_or(AppError::NotFound)
    }
}

/// Most nominal times one backfill may hold (the hard ceiling is 10,000).
pub const MAX_BACKFILL_ACTIONS: usize = 1_000;

// --- the use cases, behind a trait object --------------------------------------------------------

/// The answer of a use case, boxed so the HTTP layer holds one `Arc<dyn ScheduleUseCase>` whatever
/// the store, the calendar and the clock behind it are.
pub type ScheduleFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, AppError>> + Send + 'a>>;

/// What the HTTP layer asks of the schedules. Implemented by [`ScheduleService`].
pub trait ScheduleUseCase: Send + Sync {
    fn namespace_of<'a>(
        &'a self,
        principal: &'a Principal,
        id: ScheduleId,
    ) -> ScheduleFuture<'a, NamespaceId>;

    fn create<'a>(
        &'a self,
        principal: &'a Principal,
        namespace: NamespaceId,
        document: ScheduleDocument,
        ctx: &'a RequestContext,
    ) -> ScheduleFuture<'a, ScheduleRecord>;

    fn describe<'a>(
        &'a self,
        principal: &'a Principal,
        id: ScheduleId,
        next: u32,
        recent: u32,
    ) -> ScheduleFuture<'a, ScheduleDescription>;

    fn list<'a>(
        &'a self,
        principal: &'a Principal,
        namespace: NamespaceId,
        after: Option<ScheduleId>,
        limit: u32,
    ) -> ScheduleFuture<'a, Vec<ScheduleRecord>>;

    fn update<'a>(
        &'a self,
        principal: &'a Principal,
        id: ScheduleId,
        document: ScheduleDocument,
        expected_revision: u32,
        ctx: &'a RequestContext,
    ) -> ScheduleFuture<'a, ScheduleRecord>;

    fn delete<'a>(
        &'a self,
        principal: &'a Principal,
        id: ScheduleId,
        expected_revision: u32,
        ctx: &'a RequestContext,
    ) -> ScheduleFuture<'a, ()>;

    fn pause<'a>(
        &'a self,
        principal: &'a Principal,
        id: ScheduleId,
        note: Note,
        ctx: &'a RequestContext,
    ) -> ScheduleFuture<'a, ScheduleRecord>;

    fn resume<'a>(
        &'a self,
        principal: &'a Principal,
        id: ScheduleId,
        note: Note,
        ctx: &'a RequestContext,
    ) -> ScheduleFuture<'a, ScheduleRecord>;

    /// Asks to act on the times of a past range that have no action yet (`FR-018-011`).
    fn create_backfill<'a>(
        &'a self,
        principal: &'a Principal,
        id: ScheduleId,
        request: BackfillRequest,
        ctx: &'a RequestContext,
    ) -> ScheduleFuture<'a, BackfillRecord>;

    fn backfills<'a>(
        &'a self,
        principal: &'a Principal,
        id: ScheduleId,
    ) -> ScheduleFuture<'a, Vec<BackfillRecord>>;

    fn cancel_backfill<'a>(
        &'a self,
        principal: &'a Principal,
        id: ScheduleId,
        backfill: Uuid,
        ctx: &'a RequestContext,
    ) -> ScheduleFuture<'a, BackfillRecord>;

    /// Acts on the schedule now (`FR-018-010`); `overlap` replaces its rule for this action only.
    fn trigger<'a>(
        &'a self,
        principal: &'a Principal,
        id: ScheduleId,
        overlap: Option<Overlap>,
        key: &'a str,
        ctx: &'a RequestContext,
    ) -> ScheduleFuture<'a, ActionView>;
}
