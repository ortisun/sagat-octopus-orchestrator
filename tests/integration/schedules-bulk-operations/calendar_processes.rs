//! The same instants in separate processes (`T018-013`, `SC-018-001`, `FR-018-002`, `FR-018-003`, gap G2073): the
//! nominal times of a spec and the instants they are acted on (the jitter of the schedule and the time included) are a
//! function of the spec, the schedule's id and the time-zone database it was built with, and of nothing a process has
//! that another does not (a random seed, the zone it is told it lives in, its locale, where it was started, where the
//! system keeps its zone files). This runs the calculation of 440 cases (ten specs of every kind, with and without
//! jitter, in eleven zones: northern and southern, with a half hour and three quarters of an hour of offset, with a
//! half hour of daylight saving, and without any; each from four dates around the changes of 2026) in this process
//! and in two others that are started as this test binary and told the opposite of each other on everything the
//! environment can say, and compares the instants as instants. Nothing sleeps: the other processes are asked, they
//! answer and they end.
//!
//! The same file holds a handful of cases checked against an independent oracle, the instants that Python's `zoneinfo`
//! (the system's own zone files, not the database `chrono-tz` embeds) gives for the gaps and the folds of the southern
//! and the half-hour zones, which the table of `schedule_calendar` has for Lisbon and New York only.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::process::Command;

use sagat_application::schedule_codec::spec_from_json;
use sagat_application::schedules::upcoming;
use sagat_domain::schedules::{Ceilings, ScheduleId};
use sagat_infra_ir::schedule_calendar::SpecCalendar;
use serde_json::{Value, json};
use time::OffsetDateTime;
use uuid::Uuid;

const ZONES: [&str; 11] = [
    "Europe/Lisbon",
    "America/New_York",
    "Australia/Sydney",
    "Pacific/Auckland",
    "America/Santiago",
    "Australia/Adelaide",
    "Australia/Lord_Howe",
    "Asia/Kolkata",
    "Asia/Kathmandu",
    "America/St_Johns",
    "UTC",
];

/// 2026-03-07, 2026-03-28, 2026-09-20 and 2026-10-24, each at 00:00 UTC: the first month after each reaches a change of
/// daylight saving of the north (the United States on 8 March, Europe on 29 March and 25 October, the United States
/// on 1 November) and of the south (5 April, 27 September and 4 October).
const ANCHORS: [i64; 4] = [1_772_841_600, 1_774_656_000, 1_789_862_400, 1_792_800_000];

fn daily(hour: u32, minute: u32, zone: &str, dst: &Value, jitter_ms: u64) -> Value {
    json!({
        "calendars": [{"hour": [{"start": hour}], "minute": [{"start": minute}]}],
        "timezone": zone,
        "dst": dst,
        "jitterMs": jitter_ms,
    })
}

fn specs(zone: &str) -> Vec<(&'static str, Value)> {
    let skip = json!({"gap": "SKIP", "ambiguous": "ONCE_EARLIER"});
    let shift_twice = json!({"gap": "SHIFT_FORWARD", "ambiguous": "TWICE"});
    let weekdays = json!([{"dayOfWeek": [{"start": 1, "end": 5}], "hour": [{"start": 9}]}]);
    vec![
        ("daily 02:30, a gap skipped", daily(2, 30, zone, &skip, 0)),
        (
            "daily 02:30, a gap shifted, with jitter",
            daily(2, 30, zone, &shift_twice, 45_000),
        ),
        (
            "daily 01:30, a fold twice",
            daily(1, 30, zone, &shift_twice, 0),
        ),
        (
            "weekdays 09:00 with jitter",
            json!({"calendars": weekdays, "timezone": zone, "jitterMs": 600_000}),
        ),
        (
            "the first and the fifteenth",
            json!({
                "calendars": [{"dayOfMonth": [{"start": 1}, {"start": 15}], "hour": [{"start": 0}]}],
                "timezone": zone,
            }),
        ),
        (
            "every ninety minutes with an offset and jitter",
            json!({
                "intervals": [{"everyMs": 5_400_000, "offsetMs": 600_000}],
                "timezone": zone,
                "jitterMs": 30_000,
            }),
        ),
        (
            "hourly with jitter",
            json!({"intervals": [{"everyMs": 3_600_000}], "timezone": zone, "jitterMs": 600_000}),
        ),
        (
            "cron 02:30",
            json!({"cron": ["30 2 * * *"], "timezone": zone}),
        ),
        (
            "cron every six hours with jitter",
            json!({"cron": ["0 */6 * * *"], "timezone": zone, "jitterMs": 30_000}),
        ),
        (
            "weekdays 09:00 but Mondays",
            json!({
                "calendars": weekdays,
                "exclusions": [{"dayOfWeek": [{"start": 1}]}],
                "timezone": zone,
            }),
        ),
    ]
}

struct Case {
    label: String,
    id: ScheduleId,
    spec: Value,
    after: i64,
}

/// The cases, in an order that every process builds the same.
fn cases() -> Vec<Case> {
    let mut all = Vec::new();
    for zone in ZONES {
        for (label, spec) in specs(zone) {
            for anchor in ANCHORS {
                // a stable id for each case, as a schedule has one for its life: the jitter is a function of it
                let n = u128::try_from(all.len()).expect("small");
                all.push(Case {
                    label: format!("{zone}: {label}, after {anchor}"),
                    id: ScheduleId::new(Uuid::from_u128(
                        0x0180_0000_0000_7000_8000_0000_0000_0000 | n,
                    )),
                    spec: spec.clone(),
                    after: anchor,
                });
            }
        }
    }
    all
}

/// `(nominal, actual)` in milliseconds since the epoch of the next thirty times of a case.
fn compute(case: &Case) -> Vec<(i64, i64)> {
    let spec = spec_from_json(&case.spec, &Ceilings::HARD).expect("the spec decodes");
    let after = OffsetDateTime::from_unix_timestamp(case.after).expect("an instant");
    upcoming(&SpecCalendar, case.id, &spec, after, 30)
        .unwrap_or_else(|error| panic!("{}: {error:?}", case.label))
        .into_iter()
        .map(|(nominal, actual)| {
            (
                i64::try_from(nominal.unix_timestamp_nanos() / 1_000_000).expect("ms"),
                i64::try_from(actual.unix_timestamp_nanos() / 1_000_000).expect("ms"),
            )
        })
        .collect()
}

fn render(times: &[(i64, i64)]) -> String {
    times
        .iter()
        .map(|(nominal, actual)| format!("{nominal}:{actual}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn parse(line: &str) -> Option<(usize, Vec<(i64, i64)>)> {
    // the harness prints the name of the test before what it writes, on the same line: the first case follows it
    let mut fields = line[line.find("CASE\t")?..]
        .strip_prefix("CASE\t")?
        .splitn(2, '\t');
    let index = fields.next()?.parse().ok()?;
    let times = fields.next()?;
    let times = if times.is_empty() {
        Vec::new()
    } else {
        times
            .split(',')
            .map(|pair| {
                let (nominal, actual) = pair.split_once(':').expect("a pair");
                (
                    nominal.parse().expect("an instant"),
                    actual.parse().expect("an instant"),
                )
            })
            .collect()
    };
    Some((index, times))
}

/// The test the other processes run: it computes every case and prints what it found, and it is ignored so that
/// nothing but the test that starts it runs it.
#[test]
#[ignore = "started as a child process by the test that compares what it prints"]
fn a_process_asked_for_the_instants_prints_them() {
    for (index, case) in cases().iter().enumerate() {
        println!("CASE\t{index}\t{}", render(&compute(case)));
    }
}

/// What a process of the engine can be told that another is not.
struct Env {
    name: &'static str,
    vars: &'static [(&'static str, &'static str)],
    dir: &'static str,
}

fn ask(env: &Env) -> BTreeMap<usize, Vec<(i64, i64)>> {
    let output = Command::new(std::env::current_exe().expect("this test binary"))
        .args([
            "--ignored",
            "--exact",
            "calendar_processes::a_process_asked_for_the_instants_prints_them",
            "--nocapture",
            "--test-threads=1",
        ])
        .env_clear()
        .envs(env.vars.iter().copied())
        .current_dir(env.dir)
        .output()
        .unwrap_or_else(|error| panic!("{}: the process did not start: {error}", env.name));
    assert!(
        output.status.success(),
        "{}: the process failed: {}\n{}",
        env.name,
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse)
        .collect()
}

#[test]
fn processes_told_the_opposite_of_each_other_compute_the_same_instants_as_this_one() {
    let all = cases();
    assert!(all.len() >= 200, "{} cases", all.len());
    let here: BTreeMap<usize, Vec<(i64, i64)>> = all
        .iter()
        .enumerate()
        .map(|(index, case)| (index, compute(case)))
        .collect();
    assert!(
        here.values().all(|times| times.len() >= 10),
        "every case has times to compare"
    );
    // jitter is part of what is compared: some actual time is not its nominal time, and some is
    let moved = here
        .values()
        .flatten()
        .filter(|(nominal, actual)| nominal != actual)
        .count();
    assert!(moved > 1_000, "jitter moves {moved} instants");
    let total: usize = here.values().map(Vec::len).sum();
    println!("{} cases, {total} instants", all.len());

    let others = [
        Env {
            name: "UTC and the C locale, where nothing is installed",
            vars: &[
                ("TZ", "UTC"),
                ("LC_ALL", "C"),
                ("TZDIR", "/nonexistent/zoneinfo"),
                ("RUST_BACKTRACE", "0"),
            ],
            dir: "/",
        },
        Env {
            name: "the other side of the date line, in Turkish, with a zone directory that is not one",
            vars: &[
                ("TZ", "Pacific/Kiritimati"),
                ("LANG", "tr_TR.UTF-8"),
                ("LC_ALL", "tr_TR.UTF-8"),
                ("TZDIR", "/usr"),
                ("RUST_BACKTRACE", "1"),
                ("RUST_TEST_THREADS", "7"),
                ("SAGAT_TEST_AS_RUNTIME_ROLE", "1"),
            ],
            dir: "/usr",
        },
    ];
    for env in &others {
        let there = ask(env);
        assert_eq!(
            there.len(),
            all.len(),
            "{}: the process answered every case",
            env.name
        );
        for (index, case) in all.iter().enumerate() {
            assert_eq!(
                there.get(&index),
                here.get(&index),
                "{}: {}",
                env.name,
                case.label
            );
        }
    }
}

// --- an oracle that does not share the database -----------------------------------------------------------------

/// The next `count` nominal times of a calendar that names one local time each day, from `after`.
fn daily_times(
    zone: &str,
    hour: u32,
    minute: u32,
    dst: &Value,
    after: i64,
    count: usize,
) -> Vec<i64> {
    let spec = spec_from_json(&daily(hour, minute, zone, dst, 0), &Ceilings::HARD).expect("spec");
    let after = OffsetDateTime::from_unix_timestamp(after).expect("an instant");
    sagat_application::schedules::ScheduleCalendar::occurrences(
        &SpecCalendar,
        &spec,
        after,
        None,
        count,
    )
    .expect("occurrences")
    .into_iter()
    .map(OffsetDateTime::unix_timestamp)
    .collect()
}

const SKIP_ONCE: fn() -> Value = || json!({"gap": "SKIP", "ambiguous": "ONCE_EARLIER"});
const SHIFT_TWICE: fn() -> Value = || json!({"gap": "SHIFT_FORWARD", "ambiguous": "TWICE"});
const SHIFT_ONCE: fn() -> Value = || json!({"gap": "SHIFT_FORWARD", "ambiguous": "ONCE_EARLIER"});

#[test]
fn a_gap_of_the_south_is_skipped_or_shifted_to_the_first_instant_after_it() {
    // Sydney, 4 October 2026: 02:00 AEST (UTC+10) becomes 03:00 AEDT (UTC+11), so 02:30 does not exist
    let after = 1_790_856_000; // 2026-10-01 12:00 UTC
    assert_eq!(
        daily_times("Australia/Sydney", 2, 30, &SKIP_ONCE(), after, 5),
        [
            1_790_872_200,
            1_790_958_600,
            1_791_127_800,
            1_791_214_200,
            1_791_300_600
        ],
        "02:30 is not named that day"
    );
    // the transition is at 16:00 UTC on the 3rd: the first instant after the gap
    assert_eq!(
        daily_times("Australia/Sydney", 2, 30, &SHIFT_ONCE(), after, 4),
        [1_790_872_200, 1_790_958_600, 1_791_043_200, 1_791_127_800],
    );
    // Auckland, 27 September 2026: 02:00 NZST (UTC+12) becomes 03:00 NZDT (UTC+13)
    assert_eq!(
        daily_times("Pacific/Auckland", 2, 30, &SHIFT_ONCE(), 1_790_300_000, 3),
        [1_790_346_600, 1_790_431_200, 1_790_515_800],
    );
    // Newfoundland, 8 March 2026: 02:00 NST (UTC-3:30) becomes 03:00 NDT (UTC-2:30)
    assert_eq!(
        daily_times("America/St_Johns", 2, 30, &SHIFT_ONCE(), 1_772_800_000, 3),
        [1_772_863_200, 1_772_947_800, 1_773_032_400],
    );
}

#[test]
fn a_gap_of_half_an_hour_is_a_gap_of_half_an_hour() {
    // Lord Howe, 4 October 2026: 02:00 (UTC+10:30) becomes 02:30 (UTC+11), so 02:15 does not exist
    let after = 1_790_900_000;
    assert_eq!(
        daily_times("Australia/Lord_Howe", 2, 15, &SKIP_ONCE(), after, 2),
        [1_790_955_900, 1_791_126_900],
    );
    // the first instant after the gap is 02:30 local, 15:30 UTC on the 3rd
    assert_eq!(
        daily_times("Australia/Lord_Howe", 2, 15, &SHIFT_ONCE(), after, 3),
        [1_790_955_900, 1_791_041_400, 1_791_126_900],
    );
    // Adelaide, 4 October 2026: 02:00 ACST (UTC+9:30) becomes 03:00 ACDT (UTC+10:30)
    assert_eq!(
        daily_times("Australia/Adelaide", 2, 30, &SHIFT_ONCE(), 1_790_900_000, 3),
        [1_790_960_400, 1_791_045_000, 1_791_129_600],
    );
}

#[test]
fn a_fold_of_the_south_is_taken_once_or_twice() {
    // Sydney, 5 April 2026: 03:00 AEDT (UTC+11) becomes 02:00 AEST (UTC+10), so 02:30 happens twice
    let after = 1_775_131_200; // 2026-04-02 12:00 UTC
    assert_eq!(
        daily_times("Australia/Sydney", 2, 30, &SKIP_ONCE(), after, 5),
        [
            1_775_143_800,
            1_775_230_200,
            1_775_316_600,
            1_775_406_600,
            1_775_493_000
        ],
        "the earlier of the two"
    );
    assert_eq!(
        daily_times("Australia/Sydney", 2, 30, &SHIFT_TWICE(), after, 5),
        [
            1_775_143_800,
            1_775_230_200,
            1_775_316_600,
            1_775_320_200,
            1_775_406_600
        ],
        "both of them"
    );
    // Lord Howe, 5 April 2026: 02:00 (UTC+11) becomes 01:30 (UTC+10:30), so 01:45 happens twice, half an hour apart
    assert_eq!(
        daily_times(
            "Australia/Lord_Howe",
            1,
            45,
            &SHIFT_TWICE(),
            1_775_200_000,
            3
        ),
        [1_775_227_500, 1_775_313_900, 1_775_315_700],
        "the second 01:45 is thirty minutes after the first, not an hour"
    );
    // Adelaide the same day: 03:00 ACDT (UTC+10:30) becomes 02:00 ACST (UTC+9:30)
    assert_eq!(
        daily_times(
            "Australia/Adelaide",
            2,
            30,
            &SHIFT_TWICE(),
            1_775_200_000,
            3
        ),
        [1_775_232_000, 1_775_318_400, 1_775_322_000],
    );
    // Auckland the same day: 03:00 NZDT (UTC+13) becomes 02:00 NZST (UTC+12)
    assert_eq!(
        daily_times("Pacific/Auckland", 2, 30, &SHIFT_TWICE(), 1_775_200_000, 3),
        [1_775_223_000, 1_775_309_400, 1_775_313_000],
    );
}

#[test]
fn the_zones_with_an_offset_of_a_half_hour_or_three_quarters_and_no_daylight_saving_keep_their_wall_time()
 {
    // 09:00 in Kolkata (UTC+5:30) and Kathmandu (UTC+5:45), on 3 and 4 October 2026
    assert_eq!(
        daily_times("Asia/Kolkata", 9, 0, &SKIP_ONCE(), 1_790_950_000, 2),
        [1_790_998_200, 1_791_084_600],
    );
    assert_eq!(
        daily_times("Asia/Kathmandu", 9, 0, &SKIP_ONCE(), 1_790_950_000, 2),
        [1_790_997_300, 1_791_083_700],
    );
}
