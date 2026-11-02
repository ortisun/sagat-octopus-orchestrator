//! The calendar of schedules (feature 018): what a [`ScheduleSpec`] means once it meets a time-zone
//! database.
//!
//! A spec is the union of structured calendars, feature-004 cron strings and epoch-aligned
//! intervals, minus its exclusions, inside its bounds, in one zone. Each entry becomes an ascending
//! stream of instants in milliseconds; the streams are merged in time order, a duplicate (the same
//! instant from two entries) is dropped once, and an excluded instant is dropped.
//!
//! Daylight saving follows the spec for the structured calendars: a local time inside a spring gap is
//! skipped or shifted to the first instant after the gap, and a local time that happens twice is
//! taken once (the earlier) or twice. Cron strings keep the behaviour feature 004 gave them, whatever
//! the spec says (G1013), so moving a trigger to a schedule never changes when it fires.
//!
//! Nothing here reads a clock or a random source: the result is a function of the spec, the
//! time-zone database and the arguments.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::collections::VecDeque;
use std::str::FromStr as _;

use chrono::{
    DateTime, Datelike as _, LocalResult, NaiveDate, NaiveDateTime, Offset as _, TimeZone as _,
    Timelike as _, Utc,
};
use chrono_tz::Tz;
use croner::Cron;
use sagat_application::schedules::{
    CalendarFailure, MAX_OCCURRENCES_PER_CALL, SPACING_WINDOW, ScheduleCalendar,
};
use sagat_domain::schedules::{
    Calendar, Ceilings, DstAmbiguous, DstGap, Field, Interval, ScheduleSpec,
};
use time::OffsetDateTime;

/// The production [`ScheduleCalendar`].
#[derive(Debug, Clone, Copy, Default)]
pub struct SpecCalendar;

/// The most instants one call looks at before it gives up. A spec whose exclusions remove nearly
/// everything it produces would otherwise be walked for ever.
const MAX_PULLS: usize = 400_000;
const FIRST_YEAR: i32 = 2000;
const LAST_YEAR: i32 = 2200;

fn to_ms(t: OffsetDateTime) -> i64 {
    i64::try_from(t.unix_timestamp_nanos().div_euclid(1_000_000)).unwrap_or(i64::MAX)
}

fn from_ms(ms: i64) -> Option<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000).ok()
}

fn zone_of(spec: &ScheduleSpec) -> Result<Tz, CalendarFailure> {
    spec.timezone()
        .parse::<Tz>()
        .map_err(|_| CalendarFailure::TimeZone)
}

// --- the values of a calendar -------------------------------------------------------------------

/// The values each field of a calendar accepts, sorted and without repeats.
#[derive(Debug, Clone)]
struct Sets {
    second: Vec<u32>,
    minute: Vec<u32>,
    hour: Vec<u32>,
    day_of_month: Vec<u32>,
    month: Vec<u32>,
    year: Vec<i32>,
    day_of_week: Vec<u32>,
}

fn expand(field: Option<&Field>, default: &[u32]) -> Vec<u32> {
    let Some(field) = field else {
        return default.to_vec();
    };
    let mut out: Vec<u32> = field
        .ranges()
        .iter()
        .flat_map(|r| (r.start()..=r.end()).step_by(r.step() as usize))
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

impl Sets {
    fn of(calendar: &Calendar) -> Self {
        let f = calendar.fields();
        let years = (u32::try_from(FIRST_YEAR).unwrap_or(2000)
            ..=u32::try_from(LAST_YEAR).unwrap_or(2200))
            .collect::<Vec<_>>();
        Self {
            second: expand(f.second.as_ref(), &[0]),
            minute: expand(f.minute.as_ref(), &[0]),
            hour: expand(f.hour.as_ref(), &[0]),
            day_of_month: expand(f.day_of_month.as_ref(), &(1..=31).collect::<Vec<_>>()),
            month: expand(f.month.as_ref(), &(1..=12).collect::<Vec<_>>()),
            year: expand(f.year.as_ref(), &years)
                .into_iter()
                .filter_map(|y| i32::try_from(y).ok())
                .collect(),
            day_of_week: expand(f.day_of_week.as_ref(), &(0..=6).collect::<Vec<_>>()),
        }
    }

    /// Whether the local time of `t` is one the calendar names. Milliseconds are ignored: a calendar
    /// has second resolution.
    fn matches(&self, zone: Tz, t_ms: i64) -> bool {
        let Some(local) = zone.timestamp_millis_opt(t_ms).single() else {
            return false;
        };
        self.second.binary_search(&local.second()).is_ok()
            && self.minute.binary_search(&local.minute()).is_ok()
            && self.hour.binary_search(&local.hour()).is_ok()
            && self.day_of_month.binary_search(&local.day()).is_ok()
            && self.month.binary_search(&local.month()).is_ok()
            && self.year.binary_search(&local.year()).is_ok()
            && self
                .day_of_week
                .binary_search(&local.weekday().num_days_from_sunday())
                .is_ok()
    }

    /// The first date at or after `date` whose year, month, day of month and day of week all match.
    fn next_date(&self, mut date: NaiveDate) -> Option<NaiveDate> {
        loop {
            let year = date.year();
            if year > LAST_YEAR {
                return None;
            }
            if self.year.binary_search(&year).is_err() {
                let next = self.year.iter().copied().find(|y| *y > year)?;
                date = NaiveDate::from_ymd_opt(next, 1, 1)?;
                continue;
            }
            let month = date.month();
            if self.month.binary_search(&month).is_err() {
                date = match self.month.iter().copied().find(|m| *m > month) {
                    Some(next) => NaiveDate::from_ymd_opt(year, next, 1)?,
                    None => NaiveDate::from_ymd_opt(year.checked_add(1)?, 1, 1)?,
                };
                continue;
            }
            if self.day_of_month.binary_search(&date.day()).is_ok()
                && self
                    .day_of_week
                    .binary_search(&date.weekday().num_days_from_sunday())
                    .is_ok()
            {
                return Some(date);
            }
            date = date.succ_opt()?;
        }
    }
}

// --- the streams --------------------------------------------------------------------------------

/// An ascending stream of instants in milliseconds, each strictly after the anchor it was made with.
type Stream = Box<dyn Iterator<Item = i64>>;

/// The offset of `zone` at the instant `t` seconds, in seconds east of UTC.
fn offset_at(zone: Tz, t: i64) -> Option<i32> {
    zone.timestamp_opt(t, 0)
        .single()
        .map(|dt| dt.offset().fix().local_minus_utc())
}

/// The first instant at which the local time reaches `naive` after a gap: the transition itself.
/// `None` when the answer cannot be found or does not hold.
fn gap_end(zone: Tz, naive: NaiveDateTime) -> Option<i64> {
    let guess = naive.and_utc().timestamp();
    // a gap is at most a day wide and offsets stay within 14 hours: the transition is within 30 hours
    let (mut low, mut high) = (guess - 30 * 3600, guess + 30 * 3600);
    let after = offset_at(zone, high)?;
    if offset_at(zone, low)? == after {
        return None;
    }
    while high - low > 1 {
        let mid = low + (high - low) / 2;
        if offset_at(zone, mid)? == after {
            high = mid;
        } else {
            low = mid;
        }
    }
    // `high` is the first second with the new offset; the local time just before it is before `naive`
    let before_local = zone.timestamp_opt(high - 1, 0).single()?.naive_local();
    (before_local < naive).then_some(high)
}

/// The instants of a structured calendar, day by day.
struct CalendarStream {
    zone: Tz,
    sets: Sets,
    gap: DstGap,
    ambiguous: DstAmbiguous,
    next_day: Option<NaiveDate>,
    queue: VecDeque<i64>,
    /// Everything at or before this has been given or must not be given.
    floor_ms: i64,
    /// Nothing after this is wanted.
    stop_ms: Option<i64>,
}

impl CalendarStream {
    fn new(
        calendar: &Calendar,
        spec: &ScheduleSpec,
        zone: Tz,
        floor_ms: i64,
        stop_ms: Option<i64>,
    ) -> Self {
        // one day back, so that an instant shifted forward out of a gap is not missed
        let first = zone
            .timestamp_millis_opt(floor_ms)
            .single()
            .map(|local| local.date_naive().pred_opt().unwrap_or(local.date_naive()));
        Self {
            zone,
            sets: Sets::of(calendar),
            gap: spec.dst().gap(),
            ambiguous: spec.dst().ambiguous(),
            next_day: first,
            queue: VecDeque::new(),
            floor_ms,
            stop_ms,
        }
    }

    /// The instants of one local day, ascending and without repeats.
    fn day(&self, date: NaiveDate) -> Vec<i64> {
        let mut out = Vec::new();
        for hour in &self.sets.hour {
            for minute in &self.sets.minute {
                for second in &self.sets.second {
                    let Some(naive) = date.and_hms_opt(*hour, *minute, *second) else {
                        continue;
                    };
                    match self.zone.from_local_datetime(&naive) {
                        LocalResult::Single(t) => out.push(t.timestamp_millis()),
                        LocalResult::Ambiguous(earlier, later) => {
                            out.push(earlier.timestamp_millis());
                            if self.ambiguous == DstAmbiguous::Twice {
                                out.push(later.timestamp_millis());
                            }
                        }
                        LocalResult::None => {
                            if self.gap == DstGap::ShiftForward
                                && let Some(end) = gap_end(self.zone, naive)
                            {
                                out.push(end * 1000);
                            }
                        }
                    }
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

impl Iterator for CalendarStream {
    type Item = i64;

    fn next(&mut self) -> Option<i64> {
        loop {
            if let Some(t) = self.queue.pop_front() {
                if t <= self.floor_ms {
                    continue;
                }
                if self.stop_ms.is_some_and(|stop| t > stop) {
                    self.next_day = None;
                    self.queue.clear();
                    return None;
                }
                self.floor_ms = t;
                return Some(t);
            }
            let date = self.sets.next_date(self.next_day?)?;
            // a local day starts at most 14 hours from its UTC midnight: two days of slack
            let midnight_ms = date.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis();
            if self
                .stop_ms
                .is_some_and(|stop| midnight_ms - 2 * 86_400_000 > stop)
            {
                self.next_day = None;
                return None;
            }
            self.next_day = date.succ_opt();
            self.queue.extend(self.day(date));
        }
    }
}

/// The instants of a feature-004 cron string, with the library's own daylight-saving rules.
struct CronStream {
    cron: Cron,
    zone: Tz,
    last: DateTime<Tz>,
}

impl Iterator for CronStream {
    type Item = i64;

    fn next(&mut self) -> Option<i64> {
        let next = self.cron.find_next_occurrence(&self.last, false).ok()?;
        let ms = next.timestamp().checked_mul(1000)?;
        self.last = next.with_timezone(&self.zone);
        Some(ms)
    }
}

/// `offset + k * every` milliseconds since the epoch.
struct IntervalStream {
    every: i64,
    offset: i64,
    last: i64,
}

impl Iterator for IntervalStream {
    type Item = i64;

    fn next(&mut self) -> Option<i64> {
        let k = (self.last - self.offset)
            .div_euclid(self.every)
            .checked_add(1)?;
        let t = k.checked_mul(self.every)?.checked_add(self.offset)?;
        self.last = t;
        Some(t)
    }
}

fn interval_stream(interval: &Interval, floor_ms: i64) -> Stream {
    Box::new(IntervalStream {
        every: i64::try_from(interval.every_ms()).unwrap_or(i64::MAX),
        offset: i64::try_from(interval.offset_ms()).unwrap_or(0),
        last: floor_ms,
    })
}

// --- the port -----------------------------------------------------------------------------------

impl SpecCalendar {
    /// Nominal times in milliseconds, strictly after `after_ms`, at most `limit`, none after
    /// `until_ms` (inclusive).
    fn collect(
        spec: &ScheduleSpec,
        after_ms: i64,
        until_ms: Option<i64>,
        limit: usize,
    ) -> Result<Vec<i64>, CalendarFailure> {
        if limit > MAX_OCCURRENCES_PER_CALL {
            return Err(CalendarFailure::LimitTooLarge);
        }
        let zone = zone_of(spec)?;
        // `startAt` is inclusive, so the anchor is one millisecond before it; `endAt` is exclusive
        let floor_ms = spec.start_at().map_or(after_ms, |start| {
            after_ms.max(to_ms(start).saturating_sub(1))
        });
        let stop_ms = [
            until_ms,
            spec.end_at().map(|end| to_ms(end).saturating_sub(1)),
        ]
        .into_iter()
        .flatten()
        .min();

        let mut streams: Vec<Stream> = Vec::new();
        for calendar in spec.calendars() {
            streams.push(Box::new(CalendarStream::new(
                calendar, spec, zone, floor_ms, stop_ms,
            )));
        }
        for cron in spec.cron() {
            let parsed = Cron::from_str(cron.expression()).map_err(|_| CalendarFailure::Cron)?;
            let last = Utc
                .timestamp_millis_opt(floor_ms)
                .single()
                .map(|t| t.with_timezone(&zone));
            if let Some(last) = last {
                streams.push(Box::new(CronStream {
                    cron: parsed,
                    zone,
                    last,
                }));
            }
        }
        for interval in spec.intervals() {
            streams.push(interval_stream(interval, floor_ms));
        }
        let exclusions: Vec<Sets> = spec.exclusions().iter().map(Sets::of).collect();

        let mut heap: BinaryHeap<Reverse<(i64, usize)>> = BinaryHeap::new();
        for (i, stream) in streams.iter_mut().enumerate() {
            if let Some(t) = stream.next() {
                heap.push(Reverse((t, i)));
            }
        }
        let mut out: Vec<i64> = Vec::new();
        let mut pulls = 0usize;
        while out.len() < limit && pulls < MAX_PULLS {
            let Some(Reverse((t, i))) = heap.pop() else {
                break;
            };
            pulls += 1;
            if stop_ms.is_some_and(|stop| t > stop) {
                break;
            }
            if let Some(next) = streams[i].next() {
                heap.push(Reverse((next, i)));
            }
            if out.last() == Some(&t) || exclusions.iter().any(|e| e.matches(zone, t)) {
                continue;
            }
            out.push(t);
        }
        Ok(out)
    }
}

impl ScheduleCalendar for SpecCalendar {
    fn validate(
        &self,
        spec: &ScheduleSpec,
        now: OffsetDateTime,
        ceilings: &Ceilings,
    ) -> Result<(), CalendarFailure> {
        let ahead = Self::collect(spec, to_ms(now), None, SPACING_WINDOW)?;
        if ahead.is_empty() {
            return Err(CalendarFailure::NeverFires);
        }
        let spacing = i64::try_from(ceilings.clamped().min_spacing_ms).unwrap_or(i64::MAX);
        if ahead.windows(2).any(|w| w[1] - w[0] < spacing) {
            return Err(CalendarFailure::TooFrequent);
        }
        Ok(())
    }

    fn occurrences(
        &self,
        spec: &ScheduleSpec,
        after: OffsetDateTime,
        until: Option<OffsetDateTime>,
        limit: usize,
    ) -> Result<Vec<OffsetDateTime>, CalendarFailure> {
        Ok(Self::collect(spec, to_ms(after), until.map(to_ms), limit)?
            .into_iter()
            .filter_map(from_ms)
            .collect())
    }
}
