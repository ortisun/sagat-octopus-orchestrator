//! Schedules (feature 018): the bounded shape of a spec and its policies, the outcomes an action can
//! have, and the pure decision that says what a due time becomes.
//!
//! The domain does no calendar arithmetic: it keeps the *declared* spec (ranges, cron strings,
//! intervals, exclusions, bounds, a zone name) inside its ceilings, and decides from what the pass
//! observed under the schedule row's lock — how many tracked runs are open, how many actions are
//! waiting, how many actions remain. Computing the occurrences of a spec needs a calendar and a
//! time-zone database, so it sits behind a port in an adapter (`ScheduleCalendar`, T018-013).
//!
//! Time comes in as an argument. Randomness never does: the jitter of a nominal time is a function of
//! the schedule id and the time, so every engine process computes the same instant (D1003).

use std::fmt;
use std::str::FromStr;

use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::business_ids::{BusinessId, ConflictPolicy, ReusePolicy};
use crate::definition::{WorkflowId, WorkflowVersionId};
use crate::execution::RunStatus;
use crate::triggers::{CronSpec, is_timezone_name};

/// Hard ceilings: no namespace or deployment setting goes beyond them.
pub const MAX_SPEC_ENTRIES: usize = 32;
pub const MAX_EXCLUSIONS: usize = 32;
pub const MAX_RANGES_PER_FIELD: usize = 32;
pub const MAX_COMMENT_CHARS: usize = 128;
/// The shortest spacing an operator can configure between nominal times.
pub const MIN_SPACING_FLOOR_MS: u64 = 10_000;
pub const MAX_JITTER_MS: u32 = 86_400_000;
pub const MIN_INTERVAL_MS: u64 = 10_000;
/// 366 days: the longest interval and the longest catch-up window.
pub const MAX_INTERVAL_MS: u64 = 31_622_400_000;
pub const MIN_CATCHUP_WINDOW_MS: u64 = 10_000;
pub const MAX_CATCHUP_WINDOW_MS: u64 = 31_622_400_000;
pub const MAX_BUFFER_LIMIT: u32 = 1000;
pub const MAX_TRACKING_LIMIT: u32 = 1000;
pub const MAX_CATCHUP_ACTIONS: u32 = 1000;
pub const MAX_NOTE_CHARS: usize = 512;
pub const MAX_REMAINING_ACTIONS: u32 = 1_000_000;

/// Why a spec, a policy or a value object was refused. Each one is a `VALIDATION_FAILED` at the API;
/// the meaning travels in `details.reason`, never in a new `Error.code`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ScheduleError {
    #[error("a spec needs at least one calendar, cron string or interval")]
    NoEntries,
    #[error("a spec has at most {limit} calendars, cron strings and intervals together")]
    TooManyEntries { limit: usize },
    #[error("a spec has at most {limit} exclusions")]
    TooManyExclusions { limit: usize },
    #[error("the {field} of a calendar is outside its domain")]
    FieldOutOfDomain { field: &'static str },
    #[error("a range runs forward from its start in steps of at least one")]
    BadRange,
    #[error("a field is a union of one to {MAX_RANGES_PER_FIELD} ranges")]
    BadField,
    #[error("a calendar comment has at most {MAX_COMMENT_CHARS} characters")]
    CommentTooLong,
    #[error("an interval is 10 seconds to 366 days, with a phase smaller than the interval")]
    BadInterval,
    #[error("the end of a spec must be after its start")]
    BoundsInverted,
    #[error("jitter is at most 24 hours")]
    JitterTooLarge,
    #[error("a time zone is an IANA name such as UTC or Europe/Lisbon")]
    BadTimezone,
    #[error("a cron string has the five fields of the feature-004 grammar")]
    BadCron,
    #[error("the catch-up window is 10 seconds to 366 days")]
    BadCatchupWindow,
    #[error("the buffer limit is 1 to {MAX_BUFFER_LIMIT}")]
    BadBufferLimit,
    #[error("a note has at most {MAX_NOTE_CHARS} characters")]
    NoteTooLong,
    #[error("remaining actions are 1 to {MAX_REMAINING_ACTIONS}")]
    BadRemaining,
    #[error("the nominal time field is a top-level identifier of at most 64 characters")]
    BadNominalTimeField,
    #[error("a run timeout is one second to 366 days")]
    BadRunTimeout,
    #[error(
        "a business id prefix is 1 to 160 visible ASCII characters and leaves room for the time"
    )]
    BadBusinessIdPrefix,
}

uuid_id!(
    /// Identifier of a schedule.
    ScheduleId
);

/// The effective ceilings of a namespace: the lower of the request, the namespace and the
/// deployment, never above the hard ones. The defaults are the design values of the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ceilings {
    /// Calendars, cron strings and intervals of one spec together.
    pub spec_entries: usize,
    pub exclusions: usize,
    /// Minimum gap between two nominal times; checked by the calendar adapter.
    pub min_spacing_ms: u64,
    pub buffer_limit: u32,
    /// Open runs tracked per schedule under `ALLOW_ALL`.
    pub tracking_limit: u32,
    /// Actions taken in one recovery in catch-up mode `ALL`.
    pub catchup_actions: u32,
}

impl Default for Ceilings {
    fn default() -> Self {
        Self {
            spec_entries: 10,
            exclusions: 10,
            min_spacing_ms: 60_000,
            buffer_limit: 100,
            tracking_limit: 100,
            catchup_actions: 100,
        }
    }
}

impl Ceilings {
    /// The same ceilings held inside the hard ones: above is lowered, below the floor is raised.
    #[must_use]
    pub fn clamped(self) -> Self {
        Self {
            spec_entries: self.spec_entries.clamp(1, MAX_SPEC_ENTRIES),
            exclusions: self.exclusions.min(MAX_EXCLUSIONS),
            min_spacing_ms: self.min_spacing_ms.max(MIN_SPACING_FLOOR_MS),
            buffer_limit: self.buffer_limit.clamp(1, MAX_BUFFER_LIMIT),
            tracking_limit: self.tracking_limit.clamp(1, MAX_TRACKING_LIMIT),
            catchup_actions: self.catchup_actions.clamp(1, MAX_CATCHUP_ACTIONS),
        }
    }
}

// --- the spec ---------------------------------------------------------------------------------

/// Values `start..=end` in steps of `step`; `end` equals `start` when absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    start: u32,
    end: u32,
    step: u32,
}

impl Range {
    /// # Errors
    /// [`ScheduleError::BadRange`] when `end` is before `start` or the step is zero.
    pub fn new(start: u32, end: Option<u32>, step: Option<u32>) -> Result<Self, ScheduleError> {
        let end = end.unwrap_or(start);
        let step = step.unwrap_or(1);
        if end < start || step == 0 {
            return Err(ScheduleError::BadRange);
        }
        Ok(Self { start, end, step })
    }

    #[must_use]
    pub const fn start(self) -> u32 {
        self.start
    }

    #[must_use]
    pub const fn end(self) -> u32 {
        self.end
    }

    #[must_use]
    pub const fn step(self) -> u32 {
        self.step
    }
}

/// A union of ranges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field(Vec<Range>);

impl Field {
    /// # Errors
    /// [`ScheduleError::BadField`] unless there are one to 32 ranges.
    pub fn new(ranges: Vec<Range>) -> Result<Self, ScheduleError> {
        if ranges.is_empty() || ranges.len() > MAX_RANGES_PER_FIELD {
            return Err(ScheduleError::BadField);
        }
        Ok(Self(ranges))
    }

    #[must_use]
    pub fn ranges(&self) -> &[Range] {
        &self.0
    }
}

/// The fields of a structured calendar. An absent field takes the default of the calendar adapter:
/// second, minute and hour `0`; the others every value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CalendarFields {
    pub second: Option<Field>,
    pub minute: Option<Field>,
    pub hour: Option<Field>,
    pub day_of_month: Option<Field>,
    pub month: Option<Field>,
    pub year: Option<Field>,
    /// 0 is Sunday. A day matches when both the day of month and the day of week match.
    pub day_of_week: Option<Field>,
}

/// A structured calendar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Calendar {
    fields: CalendarFields,
    comment: Option<String>,
}

impl Calendar {
    /// # Errors
    /// [`ScheduleError::FieldOutOfDomain`] when a bound leaves the domain of its field (second 0-59,
    /// minute 0-59, hour 0-23, day of month 1-31, month 1-12, year 2000-2200, day of week 0-6) and
    /// [`ScheduleError::CommentTooLong`] over 128 characters.
    pub fn new(fields: CalendarFields, comment: Option<&str>) -> Result<Self, ScheduleError> {
        let checked: [(&'static str, &Option<Field>, u32, u32); 7] = [
            ("second", &fields.second, 0, 59),
            ("minute", &fields.minute, 0, 59),
            ("hour", &fields.hour, 0, 23),
            ("dayOfMonth", &fields.day_of_month, 1, 31),
            ("month", &fields.month, 1, 12),
            ("year", &fields.year, 2000, 2200),
            ("dayOfWeek", &fields.day_of_week, 0, 6),
        ];
        for (name, field, min, max) in checked {
            let outside = field
                .iter()
                .flat_map(Field::ranges)
                .any(|r| r.start() < min || r.end() > max);
            if outside {
                return Err(ScheduleError::FieldOutOfDomain { field: name });
            }
        }
        if comment.is_some_and(|c| c.chars().count() > MAX_COMMENT_CHARS) {
            return Err(ScheduleError::CommentTooLong);
        }
        Ok(Self {
            fields,
            comment: comment.map(str::to_owned),
        })
    }

    #[must_use]
    pub fn fields(&self) -> &CalendarFields {
        &self.fields
    }

    #[must_use]
    pub fn comment(&self) -> Option<&str> {
        self.comment.as_deref()
    }
}

/// Every `every_ms`, aligned to the Unix epoch plus `offset_ms`. Not evaluated in the zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interval {
    every_ms: u64,
    offset_ms: u64,
}

impl Interval {
    /// # Errors
    /// [`ScheduleError::BadInterval`] unless `every_ms` is 10 s to 366 d and the phase is smaller.
    pub fn new(every_ms: u64, offset_ms: u64) -> Result<Self, ScheduleError> {
        if !(MIN_INTERVAL_MS..=MAX_INTERVAL_MS).contains(&every_ms) || offset_ms >= every_ms {
            return Err(ScheduleError::BadInterval);
        }
        Ok(Self {
            every_ms,
            offset_ms,
        })
    }

    #[must_use]
    pub const fn every_ms(self) -> u64 {
        self.every_ms
    }

    #[must_use]
    pub const fn offset_ms(self) -> u64 {
        self.offset_ms
    }
}

/// A local time that does not exist (a spring-forward gap).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DstGap {
    /// No action. The default, and what a feature-004 cron trigger does.
    #[default]
    Skip,
    /// The first valid instant after the gap.
    ShiftForward,
}

/// A local time that happens twice (a fall-back overlap).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DstAmbiguous {
    #[default]
    OnceEarlier,
    Twice,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Dst {
    gap: DstGap,
    ambiguous: DstAmbiguous,
}

impl Dst {
    #[must_use]
    pub const fn new(gap: DstGap, ambiguous: DstAmbiguous) -> Self {
        Self { gap, ambiguous }
    }

    #[must_use]
    pub const fn gap(self) -> DstGap {
        self.gap
    }

    #[must_use]
    pub const fn ambiguous(self) -> DstAmbiguous {
        self.ambiguous
    }
}

/// What a caller declares; [`ScheduleSpec::new`] checks it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpecParts {
    pub calendars: Vec<Calendar>,
    /// Five-field strings of the feature-004 grammar, evaluated in the spec's zone.
    pub cron: Vec<String>,
    pub intervals: Vec<Interval>,
    /// A nominal time matched by an exclusion is not an occurrence.
    pub exclusions: Vec<Calendar>,
    pub start_at: Option<OffsetDateTime>,
    /// Exclusive; no action is taken at or after it, jitter included.
    pub end_at: Option<OffsetDateTime>,
    /// `UTC` when absent.
    pub timezone: Option<String>,
    pub jitter_ms: u32,
    pub dst: Dst,
}

/// The union of calendars, cron strings and intervals, minus the exclusions, inside the bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleSpec {
    calendars: Vec<Calendar>,
    cron: Vec<CronSpec>,
    intervals: Vec<Interval>,
    exclusions: Vec<Calendar>,
    start_at: Option<OffsetDateTime>,
    end_at: Option<OffsetDateTime>,
    timezone: String,
    jitter_ms: u32,
    dst: Dst,
}

impl ScheduleSpec {
    /// Checks everything that needs no calendar: the entry ceilings, the zone name, the cron
    /// grammar, the bounds and the jitter. That the zone exists, that something fires inside the
    /// bounds and that no two occurrences are closer than the minimum spacing need the calendar and
    /// are checked by its adapter.
    ///
    /// # Errors
    /// The [`ScheduleError`] naming the first rule broken.
    pub fn new(parts: SpecParts, ceilings: &Ceilings) -> Result<Self, ScheduleError> {
        let ceilings = ceilings.clamped();
        let entries = parts.calendars.len() + parts.cron.len() + parts.intervals.len();
        if entries == 0 {
            return Err(ScheduleError::NoEntries);
        }
        if entries > ceilings.spec_entries {
            return Err(ScheduleError::TooManyEntries {
                limit: ceilings.spec_entries,
            });
        }
        if parts.exclusions.len() > ceilings.exclusions {
            return Err(ScheduleError::TooManyExclusions {
                limit: ceilings.exclusions,
            });
        }
        let timezone = parts.timezone.unwrap_or_else(|| "UTC".to_owned());
        if !is_timezone_name(&timezone) {
            return Err(ScheduleError::BadTimezone);
        }
        let cron = parts
            .cron
            .iter()
            .map(|expression| {
                CronSpec::parse(expression, &timezone).map_err(|_| ScheduleError::BadCron)
            })
            .collect::<Result<Vec<_>, _>>()?;
        if let (Some(start), Some(end)) = (parts.start_at, parts.end_at)
            && end <= start
        {
            return Err(ScheduleError::BoundsInverted);
        }
        if parts.jitter_ms > MAX_JITTER_MS {
            return Err(ScheduleError::JitterTooLarge);
        }
        Ok(Self {
            calendars: parts.calendars,
            cron,
            intervals: parts.intervals,
            exclusions: parts.exclusions,
            start_at: parts.start_at,
            end_at: parts.end_at,
            timezone,
            jitter_ms: parts.jitter_ms,
            dst: parts.dst,
        })
    }

    #[must_use]
    pub fn calendars(&self) -> &[Calendar] {
        &self.calendars
    }

    #[must_use]
    pub fn cron(&self) -> &[CronSpec] {
        &self.cron
    }

    #[must_use]
    pub fn intervals(&self) -> &[Interval] {
        &self.intervals
    }

    #[must_use]
    pub fn exclusions(&self) -> &[Calendar] {
        &self.exclusions
    }

    #[must_use]
    pub const fn start_at(&self) -> Option<OffsetDateTime> {
        self.start_at
    }

    #[must_use]
    pub const fn end_at(&self) -> Option<OffsetDateTime> {
        self.end_at
    }

    #[must_use]
    pub fn timezone(&self) -> &str {
        &self.timezone
    }

    #[must_use]
    pub const fn jitter_ms(&self) -> u32 {
        self.jitter_ms
    }

    #[must_use]
    pub const fn dst(&self) -> Dst {
        self.dst
    }
}

// --- policies and small values ----------------------------------------------------------------

/// What happens to a due time while an earlier run of the schedule is still open. There is no
/// terminate variant (D1004): stopping without draining attempts and compensations would break
/// constitution III.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Overlap {
    #[default]
    Skip,
    BufferOne,
    BufferAll,
    CancelOther,
    AllowAll,
}

/// Which missed times are acted on after an outage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CatchupMode {
    /// The earliest missed time in the window; the others become one `COALESCED` summary.
    #[default]
    One,
    /// Each missed time in order, up to the per-recovery ceiling.
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policies {
    overlap: Overlap,
    catchup_window_ms: u64,
    catchup_mode: CatchupMode,
    pause_on_failure: bool,
    buffer_limit: u32,
}

impl Default for Policies {
    fn default() -> Self {
        Self {
            overlap: Overlap::Skip,
            catchup_window_ms: MAX_CATCHUP_WINDOW_MS,
            catchup_mode: CatchupMode::One,
            pause_on_failure: false,
            buffer_limit: Ceilings::default().buffer_limit,
        }
    }
}

impl Policies {
    /// # Errors
    /// [`ScheduleError::BadCatchupWindow`] outside 10 s to 366 d and
    /// [`ScheduleError::BadBufferLimit`] outside 1 to 1000. A buffer limit above the namespace
    /// ceiling is lowered to it, not refused.
    pub fn new(
        overlap: Overlap,
        catchup_window_ms: u64,
        catchup_mode: CatchupMode,
        pause_on_failure: bool,
        buffer_limit: u32,
        ceilings: &Ceilings,
    ) -> Result<Self, ScheduleError> {
        if !(MIN_CATCHUP_WINDOW_MS..=MAX_CATCHUP_WINDOW_MS).contains(&catchup_window_ms) {
            return Err(ScheduleError::BadCatchupWindow);
        }
        if !(1..=MAX_BUFFER_LIMIT).contains(&buffer_limit) {
            return Err(ScheduleError::BadBufferLimit);
        }
        Ok(Self {
            overlap,
            catchup_window_ms,
            catchup_mode,
            pause_on_failure,
            buffer_limit: buffer_limit.min(ceilings.clamped().buffer_limit),
        })
    }

    #[must_use]
    pub const fn overlap(&self) -> Overlap {
        self.overlap
    }

    #[must_use]
    pub const fn catchup_window_ms(&self) -> u64 {
        self.catchup_window_ms
    }

    #[must_use]
    pub const fn catchup_mode(&self) -> CatchupMode {
        self.catchup_mode
    }

    #[must_use]
    pub const fn pause_on_failure(&self) -> bool {
        self.pause_on_failure
    }

    #[must_use]
    pub const fn buffer_limit(&self) -> u32 {
        self.buffer_limit
    }
}

/// The plain-text note of a pause or a resume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note(String);

impl Note {
    /// # Errors
    /// [`ScheduleError::NoteTooLong`] over 512 characters.
    pub fn new(value: &str) -> Result<Self, ScheduleError> {
        if value.chars().count() > MAX_NOTE_CHARS {
            return Err(ScheduleError::NoteTooLong);
        }
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// How many more `SCHEDULED` actions may start a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemainingActions(u32);

impl RemainingActions {
    /// # Errors
    /// [`ScheduleError::BadRemaining`] outside 1 to 1,000,000.
    pub fn new(value: u32) -> Result<Self, ScheduleError> {
        if !(1..=MAX_REMAINING_ACTIONS).contains(&value) {
            return Err(ScheduleError::BadRemaining);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

// --- what a schedule starts ---------------------------------------------------------------------

/// Which version of the workflow each action starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionPolicy {
    /// The version that is active when the action is taken.
    Active,
    /// Always this version.
    Pinned(WorkflowVersionId),
}

/// The longest prefix of a business id made by an action: with its dash and the 20 characters of a
/// UTC time in seconds it stays inside the 200 of a business id.
pub const MAX_BUSINESS_ID_PREFIX_CHARS: usize = 160;
pub const MIN_RUN_TIMEOUT_MS: u64 = 1_000;
pub const MAX_RUN_TIMEOUT_MS: u64 = 31_622_400_000;
const MAX_NOMINAL_TIME_FIELD_CHARS: usize = 64;

/// What a target declares; [`ScheduleTarget::new`] checks it. The input of the run is a payload
/// stored beside the revision, not part of the target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetParts {
    pub workflow: WorkflowId,
    pub version: VersionPolicy,
    /// A declared top-level input property that receives the nominal time (RFC 3339 UTC).
    pub nominal_time_field: Option<String>,
    pub run_timeout_ms: Option<u64>,
    /// The runs of the schedule get the business id `<prefix>-<nominal time>`.
    pub business_id_prefix: Option<String>,
    pub business_id_conflict: ConflictPolicy,
    pub business_id_reuse: ReusePolicy,
}

/// The business id an action gives its run, and what happens when it is taken.
#[derive(Clone, PartialEq, Eq)]
pub struct TargetBusinessId {
    prefix: String,
    conflict: ConflictPolicy,
    reuse: ReusePolicy,
}

/// The prefix is customer text and stays out of logs.
impl fmt::Debug for TargetBusinessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TargetBusinessId")
            .field("conflict", &self.conflict)
            .field("reuse", &self.reuse)
            .finish_non_exhaustive()
    }
}

impl TargetBusinessId {
    #[must_use]
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    #[must_use]
    pub const fn conflict(&self) -> ConflictPolicy {
        self.conflict
    }

    #[must_use]
    pub const fn reuse(&self) -> ReusePolicy {
        self.reuse
    }

    /// The business id of the action taken at `nominal_time` (already rendered as RFC 3339 UTC).
    ///
    /// # Errors
    /// [`ScheduleError::BadBusinessIdPrefix`] when the result is not a legal business id: it is
    /// refused, never truncated.
    pub fn for_nominal_time(&self, nominal_time: &str) -> Result<BusinessId, ScheduleError> {
        BusinessId::new(&format!("{}-{nominal_time}", self.prefix))
            .map_err(|_| ScheduleError::BadBusinessIdPrefix)
    }
}

/// The workflow a schedule runs and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleTarget {
    workflow: WorkflowId,
    version: VersionPolicy,
    nominal_time_field: Option<String>,
    run_timeout_ms: Option<u64>,
    business_id: Option<TargetBusinessId>,
}

impl ScheduleTarget {
    /// # Errors
    /// [`ScheduleError::BadNominalTimeField`], [`ScheduleError::BadRunTimeout`] or
    /// [`ScheduleError::BadBusinessIdPrefix`].
    pub fn new(parts: TargetParts) -> Result<Self, ScheduleError> {
        if let Some(field) = &parts.nominal_time_field {
            let mut chars = field.chars();
            let head_ok = chars
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
            if !head_ok
                || field.chars().count() > MAX_NOMINAL_TIME_FIELD_CHARS
                || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return Err(ScheduleError::BadNominalTimeField);
            }
        }
        if parts
            .run_timeout_ms
            .is_some_and(|ms| !(MIN_RUN_TIMEOUT_MS..=MAX_RUN_TIMEOUT_MS).contains(&ms))
        {
            return Err(ScheduleError::BadRunTimeout);
        }
        let business_id = match parts.business_id_prefix {
            None => None,
            Some(prefix) => {
                if prefix.chars().count() > MAX_BUSINESS_ID_PREFIX_CHARS
                    || BusinessId::new(&prefix).is_err()
                {
                    return Err(ScheduleError::BadBusinessIdPrefix);
                }
                Some(TargetBusinessId {
                    prefix,
                    conflict: parts.business_id_conflict,
                    reuse: parts.business_id_reuse,
                })
            }
        };
        Ok(Self {
            workflow: parts.workflow,
            version: parts.version,
            nominal_time_field: parts.nominal_time_field,
            run_timeout_ms: parts.run_timeout_ms,
            business_id,
        })
    }

    #[must_use]
    pub const fn workflow(&self) -> WorkflowId {
        self.workflow
    }

    #[must_use]
    pub const fn version(&self) -> VersionPolicy {
        self.version
    }

    #[must_use]
    pub fn nominal_time_field(&self) -> Option<&str> {
        self.nominal_time_field.as_deref()
    }

    #[must_use]
    pub const fn run_timeout_ms(&self) -> Option<u64> {
        self.run_timeout_ms
    }

    #[must_use]
    pub const fn business_id(&self) -> Option<&TargetBusinessId> {
        self.business_id.as_ref()
    }
}

// --- outcomes ---------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    Scheduled,
    Triggered,
    Backfill,
}

impl ActionKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Scheduled => "SCHEDULED",
            Self::Triggered => "TRIGGERED",
            Self::Backfill => "BACKFILL",
        }
    }

    /// Scheduled and backfill actions share the spec's nominal times; trigger-now never shares an
    /// identity with a scheduled time (FR-018-004).
    #[must_use]
    pub const fn origin(self) -> Origin {
        match self {
            Self::Scheduled | Self::Backfill => Origin::Spec,
            Self::Triggered => Origin::Manual,
        }
    }
}

/// Part of the identity of an action: at most one action exists per tenant, schedule, origin and
/// nominal time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Spec,
    Manual,
}

impl Origin {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Spec => "SPEC",
            Self::Manual => "MANUAL",
        }
    }
}

/// What became of a nominal time. The set only grows; a client treats an unknown one as "another
/// outcome".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Started,
    Buffered,
    SkippedOverlap,
    DroppedBufferFull,
    DroppedDeleted,
    Coalesced,
    MissedCatchupWindow,
    AlreadyActed,
    AuthorityRevoked,
    BusinessIdConflict,
}

impl Outcome {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Started => "STARTED",
            Self::Buffered => "BUFFERED",
            Self::SkippedOverlap => "SKIPPED_OVERLAP",
            Self::DroppedBufferFull => "DROPPED_BUFFER_FULL",
            Self::DroppedDeleted => "DROPPED_DELETED",
            Self::Coalesced => "COALESCED",
            Self::MissedCatchupWindow => "MISSED_CATCHUP_WINDOW",
            Self::AlreadyActed => "ALREADY_ACTED",
            Self::AuthorityRevoked => "AUTHORITY_REVOKED",
            Self::BusinessIdConflict => "BUSINESS_ID_CONFLICT",
        }
    }

    /// Every outcome but `BUFFERED`, which later becomes `STARTED`, `MISSED_CATCHUP_WINDOW` or
    /// `DROPPED_DELETED`.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        !matches!(self, Self::Buffered)
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Outcome {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "STARTED" => Self::Started,
            "BUFFERED" => Self::Buffered,
            "SKIPPED_OVERLAP" => Self::SkippedOverlap,
            "DROPPED_BUFFER_FULL" => Self::DroppedBufferFull,
            "DROPPED_DELETED" => Self::DroppedDeleted,
            "COALESCED" => Self::Coalesced,
            "MISSED_CATCHUP_WINDOW" => Self::MissedCatchupWindow,
            "ALREADY_ACTED" => Self::AlreadyActed,
            "AUTHORITY_REVOKED" => Self::AuthorityRevoked,
            "BUSINESS_ID_CONFLICT" => Self::BusinessIdConflict,
            _ => return Err(()),
        })
    }
}

/// The closed code that qualifies an outcome, where one is needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// `ALLOW_ALL` already tracks as many open runs as the ceiling allows.
    TrackingLimit,
}

impl Reason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TrackingLimit => "TRACKING_LIMIT",
        }
    }
}

// --- the decision -----------------------------------------------------------------------------

/// What the pass saw under the schedule row's lock. Open runs are read by looking at the tracked
/// runs' status, without a run lock: terminal status is monotonic, so a stale read only delays
/// (D1005).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Standing {
    pub open_runs: u32,
    /// Actions in `BUFFERED`, oldest first in the store.
    pub buffered: u32,
    /// `None` when the schedule has no limit.
    pub remaining: Option<u32>,
}

/// What a due time becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Admit a run in this transaction.
    Start,
    /// Record the action `BUFFERED`. With `cancel_open`, also owe the tracked open runs an ordinary
    /// cancellation (`CANCEL_OTHER`).
    Buffer { cancel_open: bool },
    /// Record `SKIPPED_OVERLAP`.
    Skip { reason: Option<Reason> },
    /// Record `DROPPED_BUFFER_FULL`.
    DropBufferFull,
    /// No action is left to give: the schedule acts as paused, with no next action, and records
    /// nothing.
    Exhausted,
}

impl Decision {
    /// The outcome to record, or `None` when nothing is recorded.
    #[must_use]
    pub const fn outcome(self) -> Option<Outcome> {
        match self {
            Self::Start => Some(Outcome::Started),
            Self::Buffer { .. } => Some(Outcome::Buffered),
            Self::Skip { .. } => Some(Outcome::SkippedOverlap),
            Self::DropBufferFull => Some(Outcome::DroppedBufferFull),
            Self::Exhausted => None,
        }
    }
}

/// Decides a due `SCHEDULED` time.
///
/// An action that is waiting goes before a new one (a new time never starts ahead of it), and it
/// keeps one of the remaining actions for itself (D1042). `CANCEL_OTHER` keeps one action waiting
/// whatever the buffer limit (D1041).
#[must_use]
pub fn decide(policies: &Policies, standing: &Standing, ceilings: &Ceilings) -> Decision {
    let ceilings = ceilings.clamped();
    if let Some(remaining) = standing.remaining
        && remaining.saturating_sub(standing.buffered) == 0
    {
        return Decision::Exhausted;
    }
    let idle = standing.open_runs == 0 && standing.buffered == 0;
    match policies.overlap() {
        Overlap::Skip => {
            // `SKIP` never buffers, but an update can leave actions waiting from a former policy.
            if idle {
                Decision::Start
            } else {
                Decision::Skip { reason: None }
            }
        }
        Overlap::AllowAll => {
            if standing.open_runs >= ceilings.tracking_limit {
                Decision::Skip {
                    reason: Some(Reason::TrackingLimit),
                }
            } else {
                Decision::Start
            }
        }
        Overlap::BufferOne => {
            if idle {
                Decision::Start
            } else if standing.buffered >= 1 {
                Decision::DropBufferFull
            } else {
                Decision::Buffer { cancel_open: false }
            }
        }
        Overlap::BufferAll => {
            if idle {
                Decision::Start
            } else if standing.buffered >= policies.buffer_limit().min(ceilings.buffer_limit) {
                Decision::DropBufferFull
            } else {
                Decision::Buffer { cancel_open: false }
            }
        }
        Overlap::CancelOther => {
            if idle {
                Decision::Start
            } else if standing.buffered >= 1 {
                Decision::DropBufferFull
            } else {
                Decision::Buffer { cancel_open: true }
            }
        }
    }
}

/// What to do with the actions waiting for their turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Release {
    Nothing,
    /// Tracked runs are still open.
    Wait,
    /// Start the oldest waiting action. Its remaining action was reserved when it was buffered.
    StartOldest,
}

#[must_use]
pub const fn release(standing: &Standing) -> Release {
    if standing.buffered == 0 {
        Release::Nothing
    } else if standing.open_runs == 0 {
        Release::StartOldest
    } else {
        Release::Wait
    }
}

/// Whether a run that ended this way pauses a schedule with `pauseOnFailure` (not `CANCELLED`, which
/// somebody asked for).
#[must_use]
pub const fn failure_pauses(status: RunStatus) -> bool {
    match status {
        RunStatus::Failed
        | RunStatus::TimedOut
        | RunStatus::CompensationFailed
        | RunStatus::Partial => true,
        RunStatus::Running
        | RunStatus::Waiting
        | RunStatus::Paused
        | RunStatus::Cancelling
        | RunStatus::Compensating
        | RunStatus::ManualReview
        | RunStatus::Succeeded
        | RunStatus::Cancelled
        | RunStatus::Compensated => false,
    }
}

// --- catch-up ---------------------------------------------------------------------------------

/// Several nominal times recorded as one action row: how many and the first and last of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    pub count: u32,
    pub first: OffsetDateTime,
    pub last: OffsetDateTime,
}

/// What to do with the times that fell due while the platform was away.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CatchUp {
    /// One `MISSED_CATCHUP_WINDOW` summary for the times older than the window.
    pub missed: Option<Summary>,
    /// The times to act on, in order.
    pub act: Vec<OffsetDateTime>,
    /// One `COALESCED` summary for the times that are neither missed nor acted on.
    pub coalesced: Option<Summary>,
}

fn summarize(times: &[OffsetDateTime]) -> Option<Summary> {
    let (first, last) = (times.first()?, times.last()?);
    Some(Summary {
        count: u32::try_from(times.len()).unwrap_or(u32::MAX),
        first: *first,
        last: *last,
    })
}

/// Plans a recovery over the nominal times that are due at `now`. Times in the future are not due
/// and are ignored. A time exactly one window old is still inside it.
///
/// The slice is the pass's bounded view of the due times; a pass that cannot bring them all
/// (a year of per-minute times) counts the rest itself and records its own summary.
#[must_use]
pub fn plan_catch_up(
    due: &[OffsetDateTime],
    now: OffsetDateTime,
    policies: &Policies,
    ceilings: &Ceilings,
) -> CatchUp {
    let window =
        Duration::milliseconds(i64::try_from(policies.catchup_window_ms()).unwrap_or(i64::MAX));
    let cutoff = now
        .checked_sub(window)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH);
    let mut due: Vec<OffsetDateTime> = due.iter().copied().filter(|t| *t <= now).collect();
    due.sort_unstable();
    let split = due.partition_point(|t| *t < cutoff);
    let (outside, inside) = due.split_at(split);
    let taken = match policies.catchup_mode() {
        CatchupMode::One => inside.len().min(1),
        CatchupMode::All => inside
            .len()
            .min(usize::try_from(ceilings.clamped().catchup_actions).unwrap_or(usize::MAX)),
    };
    let (act, rest) = inside.split_at(taken);
    CatchUp {
        missed: summarize(outside),
        act: act.to_vec(),
        coalesced: summarize(rest),
    }
}

// --- jitter -----------------------------------------------------------------------------------

/// The bytes the caller hashes with SHA-256 to get the digest of [`jitter_offset_ms`]: the schedule
/// id, then the nominal time in whole milliseconds since the epoch, big-endian. The domain has no
/// hash of its own; the layer that does must hash exactly these bytes, so that every engine process
/// computes the same instant.
#[must_use]
pub fn jitter_seed(schedule_id: Uuid, nominal: OffsetDateTime) -> [u8; 24] {
    let millis = nominal
        .unix_timestamp()
        .saturating_mul(1000)
        .saturating_add(i64::from(nominal.millisecond()));
    let mut seed = [0u8; 24];
    seed[..16].copy_from_slice(schedule_id.as_bytes());
    seed[16..].copy_from_slice(&millis.to_be_bytes());
    seed
}

/// The offset in `[0, bound)` milliseconds that the digest picks, where `bound` is the jitter
/// setting lowered to the gap before the next nominal time. Zero when there is no room.
#[must_use]
pub fn jitter_offset_ms(digest: &[u8; 32], jitter_ms: u32, next_gap_ms: Option<u64>) -> u64 {
    let mut bound = u64::from(jitter_ms);
    if let Some(gap) = next_gap_ms {
        bound = bound.min(gap);
    }
    if bound == 0 {
        return 0;
    }
    let mut head = [0u8; 8];
    head.copy_from_slice(&digest[..8]);
    u64::from_be_bytes(head) % bound
}

/// The instant a nominal time is acted on, or `None` when jitter pushes it to the end of the spec
/// or beyond (`endAt` is exclusive).
#[must_use]
pub fn actual_time(
    nominal: OffsetDateTime,
    offset_ms: u64,
    end_at: Option<OffsetDateTime>,
) -> Option<OffsetDateTime> {
    let offset = Duration::milliseconds(i64::try_from(offset_ms).ok()?);
    let actual = nominal.checked_add(offset)?;
    match end_at {
        Some(end) if actual >= end => None,
        Some(_) | None => Some(actual),
    }
}
