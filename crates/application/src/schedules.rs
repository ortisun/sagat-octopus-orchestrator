//! Schedules (feature 018): the ports their use cases stand on.
//!
//! Only the calendar port exists so far. It is the one piece that needs a time-zone database, so
//! the domain keeps the declared spec and an adapter (`sagat-infra-ir`) makes it mean something,
//! the way it does for the cron triggers of feature 004 ([`crate::dispatch::CronSchedule`]).

use sagat_domain::schedules::{Ceilings, ScheduleSpec};
use time::OffsetDateTime;

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
