//! Schedules (feature 018): the ports their use cases stand on.
//!
//! Only the calendar port exists so far. It is the one piece that needs a time-zone database, so
//! the domain keeps the declared spec and an adapter (`sagat-infra-ir`) makes it mean something,
//! the way it does for the cron triggers of feature 004 ([`crate::dispatch::CronSchedule`]).

use std::future::Future;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::identity::{NamespaceId, Permission, Principal, TenantId};
use sagat_domain::quota::QuotaResource;
use sagat_domain::schedules::{
    Ceilings, Note, Policies, RemainingActions, ScheduleId, ScheduleName, ScheduleSpec,
    ScheduleTarget, actual_time, jitter_offset_ms, jitter_seed,
};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::clock::Clock;
use crate::error::{AppError, StoreError};
use crate::identity::RequestContext;
use crate::quotas::QuotaLookup;

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

    /// The schedules of a namespace that have not been deleted, by id, after `after` when given.
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

    /// Deletes: the schedule stays as a tombstone, its name is free, its waiting actions become
    /// `DROPPED_DELETED`, its runs are untouched. `false` when there was nothing to delete.
    fn delete_schedule(
        &self,
        tenant: TenantId,
        id: ScheduleId,
        now: OffsetDateTime,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;
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

fn invalid(reason: &'static str, field: &str) -> AppError {
    AppError::InvalidSchedule {
        reason,
        field: Some(field.to_owned()),
    }
}

fn calendar_error(error: CalendarFailure) -> AppError {
    match error {
        CalendarFailure::TimeZone => invalid("SCHEDULE_ZONE_UNKNOWN", "spec.timezone"),
        CalendarFailure::Cron => invalid("SCHEDULE_SPEC_INVALID", "spec.cron"),
        CalendarFailure::NeverFires => invalid("SCHEDULE_NEVER_FIRES", "spec"),
        CalendarFailure::TooFrequent => invalid("SCHEDULE_TOO_FREQUENT", "spec"),
        CalendarFailure::LimitTooLarge => {
            AppError::NotAllowed("too many occurrences were asked for at once")
        }
    }
}

/// The input as the payload that is stored: an object of at most [`MAX_INPUT_BYTES`].
fn stored_input(input: &Value) -> Result<StoredInput, AppError> {
    if !input.is_object() {
        return Err(invalid("SCHEDULE_INPUT_INVALID", "action.input"));
    }
    let bytes =
        serde_json::to_vec(input).map_err(|_| invalid("SCHEDULE_INPUT_INVALID", "action.input"))?;
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

    /// The first nominal time after `after` and the instant it is acted on (jitter included). The
    /// jitter is the digest of the schedule and the nominal time, bounded by the gap to the next
    /// nominal time, so every engine process computes the same instant.
    fn next_times(
        &self,
        id: ScheduleId,
        spec: &ScheduleSpec,
        after: OffsetDateTime,
    ) -> Result<(Option<OffsetDateTime>, Option<OffsetDateTime>), AppError> {
        let Some(nominal) = self
            .calendar
            .next_after(spec, after)
            .map_err(calendar_error)?
        else {
            return Ok((None, None));
        };
        let gap_ms = self
            .calendar
            .next_after(spec, nominal)
            .map_err(calendar_error)?
            .and_then(|following| u64::try_from((following - nominal).whole_milliseconds()).ok());
        let digest: [u8; 32] = Sha256::digest(jitter_seed(id.as_uuid(), nominal)).into();
        let offset = jitter_offset_ms(&digest, spec.jitter_ms(), gap_ms);
        Ok((Some(nominal), actual_time(nominal, offset, spec.end_at())))
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
                return Err(invalid("SCHEDULE_SPEC_INVALID", "action.workflowId"));
            }
            TargetCheck::VersionMissing => {
                return Err(invalid("SCHEDULE_SPEC_INVALID", "action.versionId"));
            }
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

    /// The schedules of a namespace, by id, after `after`.
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
            .delete_schedule(principal.tenant_id(), id, now, &audit)
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
            return Err(invalid("SCHEDULE_SPEC_INVALID", "note"));
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
}
