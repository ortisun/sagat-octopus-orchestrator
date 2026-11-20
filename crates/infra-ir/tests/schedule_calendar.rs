//! The calendar of schedules (feature 018): what a spec means once it meets a time-zone database
//! (`FR-018-002`, `FR-018-003`, `SC-018-001`, `T018-013`).
//!
//! The expectations for Lisbon and New York come from an oracle written here from the daylight-saving
//! rules of those places and plain civil-calendar arithmetic, not from `chrono-tz`, so the adapter is
//! checked against something that does not share its database.
#![allow(clippy::expect_used, clippy::panic)]

use proptest::prelude::*;
use sagat_application::schedules::{CalendarFailure, ScheduleCalendar};
use sagat_domain::schedules::{
    Calendar, CalendarFields, Ceilings, Dst, DstAmbiguous, DstGap, Field, Interval, Range,
    ScheduleSpec, SpecParts,
};
use sagat_infra_ir::schedule_calendar::SpecCalendar;
use time::OffsetDateTime;

// --- civil-calendar oracle ---------------------------------------------------------------------

/// Days since 1970-01-01 of a civil date (Howard Hinnant's `days_from_civil`).
fn days(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 0 is Sunday. 1970-01-01 was a Thursday.
fn weekday(z: i64) -> i64 {
    (z + 4).rem_euclid(7)
}

fn last_sunday(y: i64, m: i64) -> i64 {
    let last = days(y, m + 1, 1) - 1;
    last - weekday(last)
}

fn nth_sunday(y: i64, m: i64, n: i64) -> i64 {
    let first = days(y, m, 1);
    first + (7 - weekday(first)) % 7 + 7 * (n - 1)
}

/// Seconds since the epoch of 09:00 local on day `z` in Europe/Lisbon: UTC+0 in winter, UTC+1 from
/// the last Sunday of March to the last Sunday of October (the changes happen at 01:00 UTC, long
/// before nine o'clock).
fn lisbon_nine(z: i64) -> i64 {
    let (y, _, _) = civil(z);
    let summer = last_sunday(y, 3) <= z && z < last_sunday(y, 10);
    z * 86_400 + 9 * 3600 - if summer { 3600 } else { 0 }
}

/// 09:00 local in America/New_York: UTC-5 in winter, UTC-4 from the second Sunday of March to the
/// first Sunday of November (the changes happen at 02:00 local, before nine o'clock).
fn new_york_nine(z: i64) -> i64 {
    let (y, _, _) = civil(z);
    let summer = nth_sunday(y, 3, 2) <= z && z < nth_sunday(y, 11, 1);
    z * 86_400 + 9 * 3600 + if summer { 4 * 3600 } else { 5 * 3600 }
}

// --- builders ---------------------------------------------------------------------------------

fn at(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(seconds).expect("instant")
}

fn at_ms(millis: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(millis) * 1_000_000).expect("instant")
}

fn secs(times: &[OffsetDateTime]) -> Vec<i64> {
    times.iter().map(|t| t.unix_timestamp()).collect()
}

fn values(values: &[u32]) -> Field {
    Field::new(
        values
            .iter()
            .map(|v| Range::new(*v, None, None).expect("range"))
            .collect(),
    )
    .expect("field")
}

fn span(start: u32, end: u32, step: u32) -> Field {
    Field::new(vec![
        Range::new(start, Some(end), Some(step)).expect("range"),
    ])
    .expect("field")
}

fn calendar(fields: CalendarFields) -> Calendar {
    Calendar::new(fields, None).expect("calendar")
}

fn daily_at(hour: u32) -> Calendar {
    calendar(CalendarFields {
        hour: Some(values(&[hour])),
        ..CalendarFields::default()
    })
}

fn spec(parts: SpecParts) -> ScheduleSpec {
    ScheduleSpec::new(parts, &Ceilings::default()).expect("spec")
}

fn in_zone(zone: &str, calendars: Vec<Calendar>) -> ScheduleSpec {
    spec(SpecParts {
        calendars,
        timezone: Some(zone.to_owned()),
        ..SpecParts::default()
    })
}

fn occurrences(spec: &ScheduleSpec, after: i64, limit: usize) -> Vec<i64> {
    secs(
        &SpecCalendar
            .occurrences(spec, at(after), None, limit)
            .expect("occurrences"),
    )
}

/// 2026-10-26, a Monday, 00:00 UTC.
fn monday() -> i64 {
    days(2026, 10, 26) * 86_400
}

// --- SC-018-001: a table of at least 200 cases per zone ----------------------------------------

#[test]
fn nine_in_the_morning_in_lisbon_follows_the_clocks_for_two_years() {
    let start = days(2025, 1, 1);
    let spec = in_zone("Europe/Lisbon", vec![daily_at(9)]);
    let got = occurrences(&spec, start * 86_400 - 1, 730);
    let expected: Vec<i64> = (start..start + 730).map(lisbon_nine).collect();
    assert_eq!(got.len(), 730);
    for (i, (g, e)) in got.iter().zip(&expected).enumerate() {
        assert_eq!(g, e, "day {i} from 2025-01-01");
    }
    // the oracle really crosses both changes of both years
    assert_eq!(
        lisbon_nine(days(2025, 3, 29)) % 86_400,
        9 * 3600,
        "still winter on Saturday"
    );
    assert_eq!(
        lisbon_nine(days(2025, 3, 30)) % 86_400,
        8 * 3600,
        "summer from Sunday 30 March 2025"
    );
}

#[test]
fn nine_in_the_morning_in_new_york_follows_the_clocks_for_two_years() {
    let start = days(2025, 1, 1);
    let spec = in_zone("America/New_York", vec![daily_at(9)]);
    let got = occurrences(&spec, start * 86_400 - 1, 730);
    let expected: Vec<i64> = (start..start + 730).map(new_york_nine).collect();
    assert_eq!(got.len(), 730);
    for (i, (g, e)) in got.iter().zip(&expected).enumerate() {
        assert_eq!(g, e, "day {i} from 2025-01-01");
    }
    assert_eq!(
        new_york_nine(days(2026, 3, 8)) % 86_400,
        13 * 3600,
        "summer from 8 March 2026"
    );
    assert_eq!(
        new_york_nine(days(2026, 11, 1)) % 86_400,
        14 * 3600,
        "winter again on 1 November 2026"
    );
}

#[test]
fn a_cron_string_agrees_with_the_calendar_of_the_same_meaning_in_both_zones() {
    for (zone, oracle) in [
        ("Europe/Lisbon", lisbon_nine as fn(i64) -> i64),
        ("America/New_York", new_york_nine as fn(i64) -> i64),
    ] {
        let by_cron = spec(SpecParts {
            cron: vec!["0 9 * * *".to_owned()],
            timezone: Some(zone.to_owned()),
            ..SpecParts::default()
        });
        let start = days(2026, 1, 1);
        let got = occurrences(&by_cron, start * 86_400 - 1, 365);
        let expected: Vec<i64> = (start..start + 365).map(oracle).collect();
        assert_eq!(got, expected, "{zone}");
    }
}

// --- the shapes of a calendar ------------------------------------------------------------------

#[test]
fn day_of_month_and_day_of_week_must_both_match() {
    // Friday the 13th of 2026
    let spec = in_zone(
        "UTC",
        vec![calendar(CalendarFields {
            day_of_month: Some(values(&[13])),
            day_of_week: Some(values(&[5])),
            year: Some(values(&[2026])),
            ..CalendarFields::default()
        })],
    );
    let got = occurrences(&spec, days(2025, 12, 31) * 86_400, 10);
    let expected = vec![
        days(2026, 2, 13) * 86_400,
        days(2026, 3, 13) * 86_400,
        days(2026, 11, 13) * 86_400,
    ];
    assert_eq!(got, expected);
}

#[test]
fn the_31st_skips_the_shorter_months() {
    let spec = in_zone(
        "UTC",
        vec![calendar(CalendarFields {
            day_of_month: Some(values(&[31])),
            year: Some(values(&[2026])),
            ..CalendarFields::default()
        })],
    );
    let months = [1, 3, 5, 7, 8, 10, 12];
    let expected: Vec<i64> = months.iter().map(|m| days(2026, *m, 31) * 86_400).collect();
    assert_eq!(
        occurrences(&spec, days(2025, 12, 31) * 86_400, 20),
        expected
    );
}

#[test]
fn ranges_with_steps_and_lists_expand_in_order() {
    let spec = in_zone(
        "UTC",
        vec![calendar(CalendarFields {
            hour: Some(span(9, 10, 1)),
            minute: Some(span(0, 59, 15)),
            second: Some(values(&[0, 30])),
            ..CalendarFields::default()
        })],
    );
    let day = days(2026, 10, 26) * 86_400;
    let got = occurrences(&spec, day - 1, 16);
    let mut expected = Vec::new();
    for hour in [9, 10] {
        for minute in [0, 15, 30, 45] {
            for second in [0, 30] {
                expected.push(day + hour * 3600 + minute * 60 + second);
            }
        }
    }
    assert_eq!(got, expected);
}

#[test]
fn a_year_outside_the_calendar_ends_the_spec() {
    let spec = in_zone(
        "UTC",
        vec![calendar(CalendarFields {
            year: Some(values(&[2026])),
            month: Some(values(&[12])),
            day_of_month: Some(values(&[31])),
            ..CalendarFields::default()
        })],
    );
    let last = days(2026, 12, 31) * 86_400;
    assert_eq!(occurrences(&spec, last - 1, 5), vec![last]);
    assert!(occurrences(&spec, last, 5).is_empty(), "nothing after 2026");
    assert_eq!(SpecCalendar.next_after(&spec, at(last)), Ok(None));
}

#[test]
fn an_interval_is_aligned_to_the_epoch_plus_its_phase() {
    let hourly = spec(SpecParts {
        intervals: vec![Interval::new(3_600_000, 900_000).expect("interval")],
        ..SpecParts::default()
    });
    let base = 1_800_000_000; // a whole hour since the epoch
    assert_eq!(
        occurrences(&hourly, base, 3),
        vec![base + 900, base + 4500, base + 8100]
    );
    assert_eq!(
        occurrences(&hourly, base + 900, 1),
        vec![base + 4500],
        "strictly after: the instant itself is not its own successor"
    );
    assert_eq!(occurrences(&hourly, base + 899, 1), vec![base + 900]);
}

#[test]
fn an_interval_keeps_millisecond_resolution_because_the_run_key_does() {
    let odd = spec(SpecParts {
        intervals: vec![Interval::new(10_500, 0).expect("interval")],
        ..SpecParts::default()
    });
    let got = SpecCalendar
        .occurrences(&odd, at_ms(0), None, 4)
        .expect("occurrences");
    assert_eq!(
        got,
        vec![at_ms(10_500), at_ms(21_000), at_ms(31_500), at_ms(42_000)]
    );
}

// --- union, exclusions, bounds ----------------------------------------------------------------

#[test]
fn the_same_instant_from_three_kinds_of_entry_is_one_occurrence() {
    let nine_utc = spec(SpecParts {
        calendars: vec![daily_at(9)],
        cron: vec!["0 9 * * *".to_owned()],
        intervals: vec![Interval::new(86_400_000, 9 * 3_600_000).expect("interval")],
        ..SpecParts::default()
    });
    let day = days(2026, 10, 26) * 86_400;
    assert_eq!(
        occurrences(&nine_utc, day, 3),
        vec![
            day + 9 * 3600,
            day + 86_400 + 9 * 3600,
            day + 2 * 86_400 + 9 * 3600
        ]
    );
}

#[test]
fn entries_of_different_kinds_interleave_in_time_order() {
    let mixed = spec(SpecParts {
        calendars: vec![daily_at(9)],
        intervals: vec![Interval::new(4 * 3_600_000, 0).expect("interval")],
        ..SpecParts::default()
    });
    let day = days(2026, 10, 26) * 86_400;
    let hours: Vec<i64> = occurrences(&mixed, day - 1, 8)
        .iter()
        .map(|t| (t - day) / 3600)
        .collect();
    assert_eq!(hours, vec![0, 4, 8, 9, 12, 16, 20, 24]);
}

#[test]
fn an_exclusion_removes_the_times_it_matches() {
    let weekdays = spec(SpecParts {
        calendars: vec![daily_at(9)],
        exclusions: vec![calendar(CalendarFields {
            hour: Some(values(&[9])),
            day_of_week: Some(values(&[0, 6])),
            ..CalendarFields::default()
        })],
        ..SpecParts::default()
    });
    let monday = monday();
    let got = occurrences(&weekdays, monday - 1, 7);
    let expected: Vec<i64> = [0, 1, 2, 3, 4, 7, 8]
        .iter()
        .map(|d| monday + d * 86_400 + 9 * 3600)
        .collect();
    assert_eq!(got, expected, "Saturday and Sunday are gone");
}

#[test]
fn an_exclusion_is_judged_in_the_zone_of_the_spec() {
    // In New York, Saturday 23:00 local is already Sunday 03:00 UTC (31 October 2026 is still on
    // daylight time): the exclusion of "Saturday 23:00" matches in the zone and in no other
    let spec = spec(SpecParts {
        calendars: vec![calendar(CalendarFields {
            hour: Some(values(&[23])),
            ..CalendarFields::default()
        })],
        exclusions: vec![calendar(CalendarFields {
            hour: Some(values(&[23])),
            day_of_week: Some(values(&[6])),
            ..CalendarFields::default()
        })],
        timezone: Some("America/New_York".to_owned()),
        ..SpecParts::default()
    });
    let monday = monday();
    let got = occurrences(&spec, monday - 1, 7);
    assert_eq!(got.len(), 7);
    let saturday_night = days(2026, 11, 1) * 86_400 + 3 * 3600;
    assert!(
        !got.contains(&saturday_night),
        "Saturday 31 October, 23:00 New York time, is excluded"
    );
    let friday_night = days(2026, 10, 31) * 86_400 + 3 * 3600;
    assert!(got.contains(&friday_night), "Friday 23:00 local is not");
}

#[test]
fn start_is_inclusive_and_end_is_exclusive() {
    let day = days(2026, 10, 26) * 86_400;
    let bounded = spec(SpecParts {
        calendars: vec![daily_at(9)],
        start_at: Some(at(day + 3 * 86_400 + 9 * 3600)),
        end_at: Some(at(day + 6 * 86_400 + 9 * 3600)),
        ..SpecParts::default()
    });
    let got = occurrences(&bounded, day - 1, 20);
    let expected: Vec<i64> = [3, 4, 5]
        .iter()
        .map(|d| day + d * 86_400 + 9 * 3600)
        .collect();
    assert_eq!(
        got, expected,
        "day 3 at 09:00 is the start (in), day 6 at 09:00 is the end (out)"
    );
}

#[test]
fn until_is_inclusive_and_the_limit_caps_the_count() {
    let spec = in_zone("UTC", vec![daily_at(9)]);
    let day = days(2026, 10, 26) * 86_400;
    let until = at(day + 2 * 86_400 + 9 * 3600);
    let got = SpecCalendar
        .occurrences(&spec, at(day - 1), Some(until), 50)
        .expect("occurrences");
    assert_eq!(got.len(), 3, "days 0, 1 and 2");
    assert_eq!(*got.last().expect("last"), until);
    assert_eq!(occurrences(&spec, day - 1, 2).len(), 2);
    assert!(occurrences(&spec, day - 1, 0).is_empty());
    assert_eq!(
        SpecCalendar.occurrences(&spec, at(day), None, 10_001),
        Err(CalendarFailure::LimitTooLarge)
    );
}

// --- daylight saving --------------------------------------------------------------------------

fn dst_spec(zone: &str, hour: u32, minute: u32, month: u32, day: u32, dst: Dst) -> ScheduleSpec {
    spec(SpecParts {
        calendars: vec![calendar(CalendarFields {
            hour: Some(values(&[hour])),
            minute: Some(values(&[minute])),
            month: Some(values(&[month])),
            day_of_month: Some(values(&[day])),
            year: Some(values(&[2026])),
            ..CalendarFields::default()
        })],
        timezone: Some(zone.to_owned()),
        dst,
        ..SpecParts::default()
    })
}

#[test]
fn a_time_in_the_spring_gap_is_skipped_by_default() {
    // 8 March 2026: New York goes from 02:00 EST straight to 03:00 EDT; 02:30 does not exist
    let skip = dst_spec("America/New_York", 2, 30, 3, 8, Dst::default());
    assert!(occurrences(&skip, days(2026, 1, 1) * 86_400, 5).is_empty());
    // Lisbon: 29 March 2026, 01:00 WET straight to 02:00 WEST
    let skip = dst_spec("Europe/Lisbon", 1, 30, 3, 29, Dst::default());
    assert!(occurrences(&skip, days(2026, 1, 1) * 86_400, 5).is_empty());
}

#[test]
fn shift_forward_moves_a_time_in_the_gap_to_the_first_instant_after_it() {
    let shift = Dst::new(DstGap::ShiftForward, DstAmbiguous::OnceEarlier);
    let new_york = dst_spec("America/New_York", 2, 30, 3, 8, shift);
    assert_eq!(
        occurrences(&new_york, days(2026, 1, 1) * 86_400, 5),
        vec![days(2026, 3, 8) * 86_400 + 7 * 3600],
        "03:00 EDT is 07:00 UTC"
    );
    let lisbon = dst_spec("Europe/Lisbon", 1, 30, 3, 29, shift);
    assert_eq!(
        occurrences(&lisbon, days(2026, 1, 1) * 86_400, 5),
        vec![days(2026, 3, 29) * 86_400 + 3600],
        "02:00 WEST is 01:00 UTC"
    );
}

#[test]
fn two_times_in_one_gap_that_shift_to_the_same_instant_are_one_occurrence() {
    let shift = Dst::new(DstGap::ShiftForward, DstAmbiguous::OnceEarlier);
    let both = spec(SpecParts {
        calendars: vec![calendar(CalendarFields {
            hour: Some(values(&[2])),
            minute: Some(values(&[10, 40])),
            month: Some(values(&[3])),
            day_of_month: Some(values(&[8])),
            year: Some(values(&[2026])),
            ..CalendarFields::default()
        })],
        timezone: Some("America/New_York".to_owned()),
        dst: shift,
        ..SpecParts::default()
    });
    assert_eq!(occurrences(&both, days(2026, 1, 1) * 86_400, 5).len(), 1);
}

#[test]
fn a_time_that_happens_twice_is_acted_on_once_by_default_and_twice_on_request() {
    // 1 November 2026: 02:00 EDT falls back to 01:00 EST, so 01:30 happens at 05:30 UTC and 06:30 UTC
    let day = days(2026, 11, 1) * 86_400;
    let once = dst_spec("America/New_York", 1, 30, 11, 1, Dst::default());
    assert_eq!(
        occurrences(&once, days(2026, 1, 1) * 86_400, 5),
        vec![day + 5 * 3600 + 1800]
    );
    let twice = dst_spec(
        "America/New_York",
        1,
        30,
        11,
        1,
        Dst::new(DstGap::Skip, DstAmbiguous::Twice),
    );
    assert_eq!(
        occurrences(&twice, days(2026, 1, 1) * 86_400, 5),
        vec![day + 5 * 3600 + 1800, day + 6 * 3600 + 1800]
    );
}

#[test]
fn a_cron_string_keeps_the_feature_004_daylight_saving_behaviour() {
    // 004 ran a cron in the zone with the library's own rules; moving it to a schedule must not change
    // when it fires, whatever the structured calendars' policy is (G1013)
    let cron = |dst| {
        spec(SpecParts {
            cron: vec!["30 2 8 3 *".to_owned()],
            timezone: Some("America/New_York".to_owned()),
            dst,
            ..SpecParts::default()
        })
    };
    let default = occurrences(&cron(Dst::default()), days(2026, 1, 1) * 86_400, 1);
    let shifted = occurrences(
        &cron(Dst::new(DstGap::ShiftForward, DstAmbiguous::Twice)),
        days(2026, 1, 1) * 86_400,
        1,
    );
    assert_eq!(default, shifted);
}

// --- validation -------------------------------------------------------------------------------

fn validate(spec: &ScheduleSpec, ceilings: &Ceilings) -> Result<(), CalendarFailure> {
    SpecCalendar.validate(spec, at(monday()), ceilings)
}

#[test]
fn a_sound_spec_is_accepted() {
    assert_eq!(
        validate(
            &in_zone("Europe/Lisbon", vec![daily_at(9)]),
            &Ceilings::default()
        ),
        Ok(())
    );
}

#[test]
fn an_unknown_zone_is_refused_here_because_only_this_side_has_the_database() {
    let spec = in_zone("Mars/Olympus_Mons", vec![daily_at(9)]);
    assert_eq!(
        validate(&spec, &Ceilings::default()),
        Err(CalendarFailure::TimeZone)
    );
    assert_eq!(
        SpecCalendar.occurrences(&spec, at(0), None, 1),
        Err(CalendarFailure::TimeZone)
    );
}

#[test]
fn a_spec_that_never_fires_is_refused() {
    let february_30 = in_zone(
        "UTC",
        vec![calendar(CalendarFields {
            month: Some(values(&[2])),
            day_of_month: Some(values(&[30])),
            ..CalendarFields::default()
        })],
    );
    assert_eq!(
        validate(&february_30, &Ceilings::default()),
        Err(CalendarFailure::NeverFires)
    );

    let ended = spec(SpecParts {
        calendars: vec![daily_at(9)],
        end_at: Some(at(monday() - 86_400)),
        ..SpecParts::default()
    });
    assert_eq!(
        validate(&ended, &Ceilings::default()),
        Err(CalendarFailure::NeverFires),
        "its end is in the past"
    );

    let excluded = spec(SpecParts {
        calendars: vec![daily_at(9)],
        exclusions: vec![calendar(CalendarFields {
            hour: Some(values(&[9])),
            ..CalendarFields::default()
        })],
        ..SpecParts::default()
    });
    assert_eq!(
        validate(&excluded, &Ceilings::default()),
        Err(CalendarFailure::NeverFires),
        "everything is excluded"
    );
}

#[test]
fn two_occurrences_closer_than_the_minimum_spacing_are_refused() {
    let every_ten_seconds = spec(SpecParts {
        intervals: vec![Interval::new(10_000, 0).expect("interval")],
        ..SpecParts::default()
    });
    assert_eq!(
        validate(&every_ten_seconds, &Ceilings::default()),
        Err(CalendarFailure::TooFrequent),
        "the default floor is 60 seconds"
    );
    let lowered = Ceilings {
        min_spacing_ms: 10_000,
        ..Ceilings::default()
    };
    assert_eq!(
        validate(&every_ten_seconds, &lowered),
        Ok(()),
        "10 seconds is the lowest an operator can set"
    );
    // a calendar can be too frequent in one place only: two times a few seconds apart
    let close = spec(SpecParts {
        calendars: vec![calendar(CalendarFields {
            second: Some(values(&[0, 20])),
            ..CalendarFields::default()
        })],
        ..SpecParts::default()
    });
    assert_eq!(
        validate(&close, &Ceilings::default()),
        Err(CalendarFailure::TooFrequent)
    );
}

#[test]
fn the_spacing_is_measured_on_the_union_of_the_entries() {
    // two intervals that together fire every 30 s, but one is excluded away at the minute mark
    let together = spec(SpecParts {
        intervals: vec![
            Interval::new(60_000, 0).expect("interval"),
            Interval::new(60_000, 30_000).expect("interval"),
        ],
        ..SpecParts::default()
    });
    assert_eq!(
        validate(&together, &Ceilings::default()),
        Err(CalendarFailure::TooFrequent)
    );
}

// --- determinism ------------------------------------------------------------------------------

fn rich_spec() -> ScheduleSpec {
    spec(SpecParts {
        calendars: vec![
            daily_at(9),
            calendar(CalendarFields {
                hour: Some(values(&[1, 2])),
                minute: Some(values(&[30])),
                day_of_week: Some(values(&[0])),
                ..CalendarFields::default()
            }),
        ],
        cron: vec!["15 */6 * * 1-5".to_owned()],
        intervals: vec![Interval::new(5 * 3_600_000, 7_000).expect("interval")],
        exclusions: vec![calendar(CalendarFields {
            hour: Some(values(&[12])),
            ..CalendarFields::default()
        })],
        timezone: Some("America/New_York".to_owned()),
        dst: Dst::new(DstGap::ShiftForward, DstAmbiguous::Twice),
        ..SpecParts::default()
    })
}

#[test]
fn two_calendars_give_the_same_answer() {
    let spec = rich_spec();
    let a = SpecCalendar
        .occurrences(&spec, at(1_790_000_000), None, 500)
        .expect("a");
    let b = SpecCalendar
        .occurrences(&spec, at(1_790_000_000), None, 500)
        .expect("b");
    assert_eq!(a, b);
    assert_eq!(a.len(), 500);
}

proptest! {
    /// Occurrences are strictly after the anchor, strictly ascending, inside the bounds, and asking
    /// in two steps gives the same list as asking at once.
    #[test]
    fn occurrences_are_ordered_bounded_and_resumable(
        after in 1_700_000_000..1_900_000_000i64,
        first in 1..40usize,
        more in 1..40usize,
    ) {
        let spec = rich_spec();
        let all = SpecCalendar.occurrences(&spec, at(after), None, first + more).expect("all");
        prop_assert_eq!(all.len(), first + more);
        prop_assert!(all[0] > at(after));
        prop_assert!(all.windows(2).all(|w| w[0] < w[1]));
        let head = SpecCalendar.occurrences(&spec, at(after), None, first).expect("head");
        prop_assert_eq!(&head[..], &all[..first]);
        let tail = SpecCalendar
            .occurrences(&spec, *head.last().expect("last"), None, more)
            .expect("tail");
        prop_assert_eq!(&tail[..], &all[first..]);
    }
}
