//! The calendar of cron triggers (`FR-004-001`): five-field schedules in an IANA time zone.
//!
//! The domain keeps a schedule as text and checks its shape; this is where it is made to mean
//! something. A schedule is checked when a workflow is published, so a trigger that could never fire
//! is refused then and not discovered at two in the morning.

use std::str::FromStr as _;

use chrono::{DateTime, TimeZone as _, Utc};
use chrono_tz::Tz;
use croner::Cron;
use sagat_application::dispatch::{CronFailure, CronSchedule};
use sagat_domain::triggers::CronSpec;
use time::OffsetDateTime;

/// The production [`CronSchedule`].
#[derive(Debug, Clone, Copy, Default)]
pub struct CronCalendar;

fn parse(spec: &CronSpec) -> Result<(Cron, Tz), CronFailure> {
    let cron = Cron::from_str(spec.expression()).map_err(|_| CronFailure::Expression)?;
    let zone = spec
        .timezone()
        .parse::<Tz>()
        .map_err(|_| CronFailure::TimeZone)?;
    Ok((cron, zone))
}

impl CronSchedule for CronCalendar {
    fn validate(&self, spec: &CronSpec) -> Result<(), CronFailure> {
        // Parsing is part of asking for the next time; a schedule that parses but never fires
        // (the 31st of February) is no schedule.
        self.next_after(spec, OffsetDateTime::now_utc()).map(|_| ())
    }

    fn next_after(
        &self,
        spec: &CronSpec,
        after: OffsetDateTime,
    ) -> Result<OffsetDateTime, CronFailure> {
        let (cron, zone) = parse(spec)?;
        let after: DateTime<Tz> = Utc
            .timestamp_opt(after.unix_timestamp(), after.nanosecond())
            .single()
            .ok_or(CronFailure::Exhausted)?
            .with_timezone(&zone);
        let next = cron
            .find_next_occurrence(&after, false)
            .map_err(|_| CronFailure::Exhausted)?;
        OffsetDateTime::from_unix_timestamp(next.timestamp()).map_err(|_| CronFailure::Exhausted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> OffsetDateTime {
        OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339).expect("time")
    }

    fn spec(expression: &str, zone: &str) -> CronSpec {
        CronSpec::parse(expression, zone).expect("spec")
    }

    #[test]
    fn the_next_time_is_the_first_strictly_after() {
        let calendar = CronCalendar;
        let every_five = spec("*/5 * * * *", "UTC");
        assert_eq!(
            calendar.next_after(&every_five, at("2026-03-01T10:02:00Z")),
            Ok(at("2026-03-01T10:05:00Z"))
        );
        assert_eq!(
            calendar.next_after(&every_five, at("2026-03-01T10:05:00Z")),
            Ok(at("2026-03-01T10:10:00Z")),
            "strictly after: a schedule is not fired twice for the same instant"
        );
        assert_eq!(
            calendar.next_after(&spec("0 9 * * MON-FRI", "UTC"), at("2026-03-06T09:00:00Z")),
            Ok(at("2026-03-09T09:00:00Z")),
            "from a Friday morning to the Monday"
        );
    }

    #[test]
    fn the_time_zone_decides_what_nine_in_the_morning_is() {
        let calendar = CronCalendar;
        let daily = spec("0 9 * * *", "America/Sao_Paulo");
        // Sao Paulo is UTC-3 all year since 2019.
        assert_eq!(
            calendar.next_after(&daily, at("2026-03-01T00:00:00Z")),
            Ok(at("2026-03-01T12:00:00Z"))
        );
        // New York changes: nine o'clock is 14:00 UTC in winter and 13:00 UTC in summer.
        let new_york = spec("0 9 * * *", "America/New_York");
        assert_eq!(
            calendar.next_after(&new_york, at("2026-03-07T00:00:00Z")),
            Ok(at("2026-03-07T14:00:00Z"))
        );
        assert_eq!(
            calendar.next_after(&new_york, at("2026-03-09T00:00:00Z")),
            Ok(at("2026-03-09T13:00:00Z")),
            "the clocks went forward on 8 March 2026"
        );
    }

    #[test]
    fn a_schedule_that_means_nothing_is_refused() {
        let calendar = CronCalendar;
        assert_eq!(
            calendar.validate(&spec("0 9 * * *", "Mars/Olympus_Mons")),
            Err(CronFailure::TimeZone)
        );
        assert_eq!(
            calendar.validate(&spec("61 * * * *", "UTC")),
            Err(CronFailure::Expression)
        );
        assert_eq!(
            calendar.validate(&spec("0 0 31 2 *", "UTC")),
            Err(CronFailure::Exhausted),
            "there is no 31st of February"
        );
        assert_eq!(
            calendar.validate(&spec("0 0 29 2 *", "UTC")),
            Ok(()),
            "but there is a 29th"
        );
    }
}
