//! Schedules (feature 018): the ports their use cases stand on.
//!
//! Only the calendar port exists so far. It is the one piece that needs a time-zone database, so
//! the domain keeps the declared spec and an adapter (`sagat-infra-ir`) makes it mean something,
//! the way it does for the cron triggers of feature 004 ([`crate::dispatch::CronSchedule`]).

use std::future::Future;

use sagat_domain::audit::AuditRecord;
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::schedules::{
    Ceilings, Policies, ScheduleId, ScheduleName, ScheduleSpec, ScheduleTarget,
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::StoreError;

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
    pub tracked_runs: Vec<Uuid>,
    pub buffered: u32,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
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

/// Storage port of the schedules. Implemented by `sagat-infra-postgres`.
pub trait ScheduleStore: Send + Sync {
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

    /// The schedules of a namespace that have not been deleted, by id, after `after` when given.
    fn schedules(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        after: Option<ScheduleId>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<ScheduleRecord>, StoreError>> + Send;
}
