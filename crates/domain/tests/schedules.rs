//! Schedules of feature 018: the shape of a spec, the policies, and the pure decision that says what a
//! due time becomes (`FR-018-002..008`, `FR-018-012`, `T018-003`).
#![allow(clippy::expect_used, clippy::panic)]

use sagat_domain::execution::RunStatus;
use sagat_domain::schedules::{
    ActionKind, Calendar, CalendarFields, CatchUp, CatchupMode, Ceilings, Decision, Dst,
    DstAmbiguous, DstGap, Field, Interval, Note, Origin, Outcome, Overlap, Policies, Range, Reason,
    Release, RemainingActions, ScheduleError, ScheduleSpec, SpecParts, Standing, Summary,
    actual_time, decide, failure_pauses, jitter_offset_ms, jitter_seed, plan_catch_up, release,
};
use time::OffsetDateTime;
use uuid::Uuid;

fn at(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(seconds).expect("instant")
}

const HOUR: i64 = 3600;
const DAY: i64 = 86_400;

fn daily_nine() -> Calendar {
    Calendar::new(
        CalendarFields {
            hour: Some(Field::new(vec![Range::new(9, None, None).expect("range")]).expect("field")),
            ..CalendarFields::default()
        },
        None,
    )
    .expect("calendar")
}

fn parts() -> SpecParts {
    SpecParts {
        calendars: vec![daily_nine()],
        ..SpecParts::default()
    }
}

// --- the spec ---------------------------------------------------------------------------------

#[test]
fn a_spec_needs_at_least_one_calendar_cron_string_or_interval() {
    let err = ScheduleSpec::new(SpecParts::default(), &Ceilings::default()).expect_err("empty");
    assert_eq!(err, ScheduleError::NoEntries);
    assert!(ScheduleSpec::new(parts(), &Ceilings::default()).is_ok());
    let only_cron = SpecParts {
        cron: vec!["0 9 * * 1-5".to_owned()],
        ..SpecParts::default()
    };
    assert!(ScheduleSpec::new(only_cron, &Ceilings::default()).is_ok());
    let only_interval = SpecParts {
        intervals: vec![Interval::new(60_000, 0).expect("interval")],
        ..SpecParts::default()
    };
    assert!(ScheduleSpec::new(only_interval, &Ceilings::default()).is_ok());
}

#[test]
fn entries_are_counted_across_calendars_cron_strings_and_intervals() {
    let many = |n: usize| SpecParts {
        calendars: vec![daily_nine(); n / 2],
        cron: vec!["0 9 * * *".to_owned(); n - n / 2],
        ..SpecParts::default()
    };
    let default = Ceilings::default();
    assert!(
        ScheduleSpec::new(many(10), &default).is_ok(),
        "the default is 10"
    );
    assert_eq!(
        ScheduleSpec::new(many(11), &default).expect_err("11"),
        ScheduleError::TooManyEntries { limit: 10 }
    );
    let raised = Ceilings {
        spec_entries: 32,
        ..Ceilings::default()
    };
    assert!(
        ScheduleSpec::new(many(32), &raised).is_ok(),
        "the hard ceiling is 32"
    );
    let absurd = Ceilings {
        spec_entries: 1000,
        ..Ceilings::default()
    };
    assert_eq!(
        ScheduleSpec::new(many(33), &absurd).expect_err("33"),
        ScheduleError::TooManyEntries { limit: 32 },
        "a ceiling above the hard one is clamped"
    );
}

#[test]
fn exclusions_have_their_own_ceiling() {
    let with = |n: usize| SpecParts {
        exclusions: vec![daily_nine(); n],
        ..parts()
    };
    assert!(ScheduleSpec::new(with(10), &Ceilings::default()).is_ok());
    assert_eq!(
        ScheduleSpec::new(with(11), &Ceilings::default()).expect_err("11"),
        ScheduleError::TooManyExclusions { limit: 10 }
    );
}

#[test]
fn a_calendar_field_must_stay_inside_its_domain() {
    let field =
        |start, end| Field::new(vec![Range::new(start, end, None).expect("range")]).expect("field");
    let cases: [(&str, CalendarFields); 7] = [
        (
            "second",
            CalendarFields {
                second: Some(field(60, None)),
                ..CalendarFields::default()
            },
        ),
        (
            "minute",
            CalendarFields {
                minute: Some(field(0, Some(60))),
                ..CalendarFields::default()
            },
        ),
        (
            "hour",
            CalendarFields {
                hour: Some(field(24, None)),
                ..CalendarFields::default()
            },
        ),
        (
            "dayOfMonth",
            CalendarFields {
                day_of_month: Some(field(0, None)),
                ..CalendarFields::default()
            },
        ),
        (
            "month",
            CalendarFields {
                month: Some(field(13, None)),
                ..CalendarFields::default()
            },
        ),
        (
            "year",
            CalendarFields {
                year: Some(field(1999, None)),
                ..CalendarFields::default()
            },
        ),
        (
            "dayOfWeek",
            CalendarFields {
                day_of_week: Some(field(7, None)),
                ..CalendarFields::default()
            },
        ),
    ];
    for (name, fields) in cases {
        let err = Calendar::new(fields, None).expect_err(name);
        assert_eq!(
            err,
            ScheduleError::FieldOutOfDomain { field: name },
            "{name}"
        );
    }
    // the edges of every domain are accepted
    let edges = CalendarFields {
        second: Some(field(0, Some(59))),
        minute: Some(field(0, Some(59))),
        hour: Some(field(0, Some(23))),
        day_of_month: Some(field(1, Some(31))),
        month: Some(field(1, Some(12))),
        year: Some(field(2000, Some(2200))),
        day_of_week: Some(field(0, Some(6))),
    };
    assert!(Calendar::new(edges, None).is_ok());
}

#[test]
fn a_range_runs_forward_in_whole_steps() {
    assert_eq!(
        Range::new(5, Some(4), None).expect_err("backwards"),
        ScheduleError::BadRange
    );
    assert_eq!(
        Range::new(0, Some(59), Some(0)).expect_err("step 0"),
        ScheduleError::BadRange
    );
    let stepped = Range::new(0, Some(59), Some(15)).expect("0-59/15");
    assert_eq!(
        (stepped.start(), stepped.end(), stepped.step()),
        (0, 59, 15)
    );
    let single = Range::new(9, None, None).expect("9");
    assert_eq!(
        (single.start(), single.end(), single.step()),
        (9, 9, 1),
        "end defaults to start"
    );
}

#[test]
fn a_field_is_a_union_of_one_to_thirty_two_ranges() {
    let r = Range::new(1, None, None).expect("range");
    assert_eq!(
        Field::new(vec![]).expect_err("empty"),
        ScheduleError::BadField
    );
    assert!(Field::new(vec![r; 32]).is_ok());
    assert_eq!(
        Field::new(vec![r; 33]).expect_err("33"),
        ScheduleError::BadField
    );
}

#[test]
fn a_calendar_comment_is_at_most_128_characters() {
    assert!(Calendar::new(CalendarFields::default(), Some(&"é".repeat(128))).is_ok());
    assert_eq!(
        Calendar::new(CalendarFields::default(), Some(&"é".repeat(129))).expect_err("long"),
        ScheduleError::CommentTooLong
    );
}

#[test]
fn an_interval_is_ten_seconds_to_a_leap_year_with_a_phase_inside_it() {
    assert!(Interval::new(10_000, 0).is_ok());
    assert_eq!(
        Interval::new(9_999, 0).expect_err("fast"),
        ScheduleError::BadInterval
    );
    assert!(Interval::new(31_622_400_000, 0).is_ok());
    assert_eq!(
        Interval::new(31_622_400_001, 0).expect_err("slow"),
        ScheduleError::BadInterval
    );
    assert_eq!(
        Interval::new(60_000, 60_000).expect_err("phase = every"),
        ScheduleError::BadInterval
    );
    let ok = Interval::new(60_000, 59_999).expect("phase below every");
    assert_eq!((ok.every_ms(), ok.offset_ms()), (60_000, 59_999));
}

#[test]
fn the_bounds_keep_start_before_end_and_jitter_within_a_day() {
    let bounded = |start, end| SpecParts {
        start_at: start,
        end_at: end,
        ..parts()
    };
    let ceilings = Ceilings::default();
    assert!(ScheduleSpec::new(bounded(Some(at(0)), Some(at(1))), &ceilings).is_ok());
    for end in [at(0), at(-1)] {
        assert_eq!(
            ScheduleSpec::new(bounded(Some(at(0)), Some(end)), &ceilings).expect_err("inverted"),
            ScheduleError::BoundsInverted
        );
    }
    let jitter = |ms| SpecParts {
        jitter_ms: ms,
        ..parts()
    };
    assert!(ScheduleSpec::new(jitter(86_400_000), &ceilings).is_ok());
    assert_eq!(
        ScheduleSpec::new(jitter(86_400_001), &ceilings).expect_err("jitter"),
        ScheduleError::JitterTooLarge
    );
}

#[test]
fn the_zone_defaults_to_utc_and_is_shared_with_the_cron_strings() {
    let spec = ScheduleSpec::new(
        SpecParts {
            cron: vec!["0  9 * * 1-5".to_owned()],
            timezone: Some("Europe/Lisbon".to_owned()),
            ..SpecParts::default()
        },
        &Ceilings::default(),
    )
    .expect("spec");
    assert_eq!(spec.timezone(), "Europe/Lisbon");
    assert_eq!(
        spec.cron()[0].expression(),
        "0 9 * * 1-5",
        "004 normalises the spaces"
    );
    assert_eq!(spec.cron()[0].timezone(), "Europe/Lisbon");
    assert_eq!(
        ScheduleSpec::new(parts(), &Ceilings::default())
            .expect("spec")
            .timezone(),
        "UTC"
    );
    for bad in ["", "not a zone", &"a".repeat(65)] {
        let err = ScheduleSpec::new(
            SpecParts {
                timezone: Some(bad.to_owned()),
                ..parts()
            },
            &Ceilings::default(),
        )
        .expect_err(bad);
        assert_eq!(err, ScheduleError::BadTimezone, "{bad:?}");
    }
}

#[test]
fn a_cron_string_follows_the_feature_004_grammar() {
    for bad in ["* * * *", "0 9 * * * *", "0 9 * * $"] {
        let err = ScheduleSpec::new(
            SpecParts {
                cron: vec![bad.to_owned()],
                ..SpecParts::default()
            },
            &Ceilings::default(),
        )
        .expect_err(bad);
        assert_eq!(err, ScheduleError::BadCron, "{bad:?}");
    }
}

#[test]
fn the_dst_policy_defaults_to_skip_and_the_earlier_instant() {
    let dst = Dst::default();
    assert_eq!(
        (dst.gap(), dst.ambiguous()),
        (DstGap::Skip, DstAmbiguous::OnceEarlier)
    );
    let spec = ScheduleSpec::new(
        SpecParts {
            dst: Dst::new(DstGap::ShiftForward, DstAmbiguous::Twice),
            ..parts()
        },
        &Ceilings::default(),
    )
    .expect("spec");
    assert_eq!(spec.dst().gap(), DstGap::ShiftForward);
    assert_eq!(spec.dst().ambiguous(), DstAmbiguous::Twice);
}

// --- the policies -----------------------------------------------------------------------------

#[test]
fn the_default_policies_skip_overlap_catch_up_one_inside_a_leap_year_and_never_pause() {
    let p = Policies::default();
    assert_eq!(p.overlap(), Overlap::Skip);
    assert_eq!(p.catchup_window_ms(), 31_622_400_000);
    assert_eq!(p.catchup_mode(), CatchupMode::One);
    assert!(!p.pause_on_failure());
    assert_eq!(p.buffer_limit(), 100);
}

#[test]
fn the_catch_up_window_is_ten_seconds_to_a_leap_year() {
    let ceilings = Ceilings::default();
    let with = |ms| Policies::new(Overlap::Skip, ms, CatchupMode::One, false, 100, &ceilings);
    assert!(with(10_000).is_ok());
    assert!(with(31_622_400_000).is_ok());
    assert_eq!(
        with(9_999).expect_err("short"),
        ScheduleError::BadCatchupWindow
    );
    assert_eq!(
        with(31_622_400_001).expect_err("long"),
        ScheduleError::BadCatchupWindow
    );
}

#[test]
fn the_buffer_limit_is_one_to_a_thousand_and_clamped_to_the_namespace_ceiling() {
    let ceilings = Ceilings {
        buffer_limit: 200,
        ..Ceilings::default()
    };
    let with = |n| {
        Policies::new(
            Overlap::BufferAll,
            31_622_400_000,
            CatchupMode::One,
            false,
            n,
            &ceilings,
        )
    };
    assert_eq!(with(0).expect_err("zero"), ScheduleError::BadBufferLimit);
    assert_eq!(
        with(1001).expect_err("over the hard ceiling"),
        ScheduleError::BadBufferLimit
    );
    assert_eq!(with(150).expect("150").buffer_limit(), 150);
    assert_eq!(
        with(500).expect("500").buffer_limit(),
        200,
        "clamped, not refused"
    );
}

#[test]
fn ceilings_above_the_hard_ones_are_clamped_and_below_the_floor_raised() {
    let c = Ceilings {
        spec_entries: 500,
        exclusions: 500,
        min_spacing_ms: 1,
        buffer_limit: 5000,
        tracking_limit: 5000,
        catchup_actions: 5000,
    }
    .clamped();
    assert_eq!(c.spec_entries, 32);
    assert_eq!(c.exclusions, 32);
    assert_eq!(c.min_spacing_ms, 10_000, "the floor is 10 s");
    assert_eq!(c.buffer_limit, 1000);
    assert_eq!(c.tracking_limit, 1000);
    assert_eq!(c.catchup_actions, 1000);
    let d = Ceilings::default();
    assert_eq!(
        (
            d.spec_entries,
            d.exclusions,
            d.min_spacing_ms,
            d.buffer_limit,
            d.tracking_limit,
            d.catchup_actions
        ),
        (10, 10, 60_000, 100, 100, 100)
    );
}

#[test]
fn a_note_is_plain_text_of_at_most_512_characters() {
    assert!(Note::new(&"é".repeat(512)).is_ok());
    assert_eq!(
        Note::new(&"é".repeat(513)).expect_err("long"),
        ScheduleError::NoteTooLong
    );
    assert_eq!(
        Note::new("maintenance window").expect("note").as_str(),
        "maintenance window"
    );
}

#[test]
fn remaining_actions_are_one_to_a_million() {
    assert!(RemainingActions::new(1).is_ok());
    assert!(RemainingActions::new(1_000_000).is_ok());
    assert_eq!(
        RemainingActions::new(0).expect_err("zero"),
        ScheduleError::BadRemaining
    );
    assert_eq!(
        RemainingActions::new(1_000_001).expect_err("over"),
        ScheduleError::BadRemaining
    );
    assert_eq!(RemainingActions::new(2).expect("two").get(), 2);
}

// --- outcomes ---------------------------------------------------------------------------------

#[test]
fn outcomes_kinds_and_origins_have_the_names_of_the_contract() {
    let outcomes = [
        (Outcome::Started, "STARTED", true),
        (Outcome::Buffered, "BUFFERED", false),
        (Outcome::SkippedOverlap, "SKIPPED_OVERLAP", true),
        (Outcome::DroppedBufferFull, "DROPPED_BUFFER_FULL", true),
        (Outcome::DroppedDeleted, "DROPPED_DELETED", true),
        (Outcome::Coalesced, "COALESCED", true),
        (Outcome::MissedCatchupWindow, "MISSED_CATCHUP_WINDOW", true),
        (Outcome::AlreadyActed, "ALREADY_ACTED", true),
        (Outcome::AuthorityRevoked, "AUTHORITY_REVOKED", true),
        (Outcome::BusinessIdConflict, "BUSINESS_ID_CONFLICT", true),
    ];
    for (outcome, name, terminal) in outcomes {
        assert_eq!(outcome.as_str(), name);
        assert_eq!(name.parse::<Outcome>(), Ok(outcome));
        assert_eq!(outcome.is_terminal(), terminal, "{name}");
    }
    assert!(
        "TERMINATED".parse::<Outcome>().is_err(),
        "there is no terminate"
    );
    assert_eq!(
        [
            ActionKind::Scheduled.as_str(),
            ActionKind::Triggered.as_str(),
            ActionKind::Backfill.as_str()
        ],
        ["SCHEDULED", "TRIGGERED", "BACKFILL"]
    );
    assert_eq!(
        [Origin::Spec.as_str(), Origin::Manual.as_str()],
        ["SPEC", "MANUAL"]
    );
    assert_eq!(ActionKind::Scheduled.origin(), Origin::Spec);
    assert_eq!(
        ActionKind::Backfill.origin(),
        Origin::Spec,
        "backfill shares the spec's nominal times"
    );
    assert_eq!(ActionKind::Triggered.origin(), Origin::Manual);
    assert_eq!(Reason::TrackingLimit.as_str(), "TRACKING_LIMIT");
}

// --- the decision -----------------------------------------------------------------------------

fn policies(overlap: Overlap, buffer_limit: u32) -> Policies {
    Policies::new(
        overlap,
        31_622_400_000,
        CatchupMode::One,
        false,
        buffer_limit,
        &Ceilings::default(),
    )
    .expect("policies")
}

fn standing(open_runs: u32, buffered: u32) -> Standing {
    Standing {
        open_runs,
        buffered,
        remaining: None,
    }
}

fn decision(overlap: Overlap, open_runs: u32, buffered: u32) -> Decision {
    decide(
        &policies(overlap, 3),
        &standing(open_runs, buffered),
        &Ceilings::default(),
    )
}

#[test]
fn skip_starts_only_when_nothing_is_open() {
    assert_eq!(decision(Overlap::Skip, 0, 0), Decision::Start);
    assert_eq!(
        decision(Overlap::Skip, 1, 0),
        Decision::Skip { reason: None }
    );
    assert_eq!(
        Decision::Skip { reason: None }.outcome(),
        Some(Outcome::SkippedOverlap)
    );
}

#[test]
fn skip_does_not_start_ahead_of_an_action_a_policy_change_left_waiting() {
    // An update from BUFFER_ALL to SKIP leaves buffered actions behind; they go first (D1043).
    assert_eq!(
        decision(Overlap::Skip, 0, 2),
        Decision::Skip { reason: None }
    );
    assert_eq!(
        decision(Overlap::Skip, 1, 2),
        Decision::Skip { reason: None }
    );
}

#[test]
fn buffer_one_keeps_one_waiting_and_drops_the_later_ones() {
    assert_eq!(decision(Overlap::BufferOne, 0, 0), Decision::Start);
    assert_eq!(
        decision(Overlap::BufferOne, 1, 0),
        Decision::Buffer { cancel_open: false }
    );
    assert_eq!(decision(Overlap::BufferOne, 1, 1), Decision::DropBufferFull);
    assert_eq!(
        decision(Overlap::BufferOne, 0, 1),
        Decision::DropBufferFull,
        "the waiting one goes first, so a new time cannot start ahead of it"
    );
}

#[test]
fn buffer_all_buffers_up_to_its_limit_in_order() {
    assert_eq!(decision(Overlap::BufferAll, 0, 0), Decision::Start);
    assert_eq!(
        decision(Overlap::BufferAll, 1, 0),
        Decision::Buffer { cancel_open: false }
    );
    assert_eq!(
        decision(Overlap::BufferAll, 1, 2),
        Decision::Buffer { cancel_open: false }
    );
    assert_eq!(
        decision(Overlap::BufferAll, 1, 3),
        Decision::DropBufferFull,
        "the limit of this test is 3"
    );
    assert_eq!(
        decision(Overlap::BufferAll, 0, 1),
        Decision::Buffer { cancel_open: false },
        "nothing is open but an earlier time is waiting for its turn"
    );
    assert_eq!(
        Decision::Buffer { cancel_open: false }.outcome(),
        Some(Outcome::Buffered)
    );
    assert_eq!(
        Decision::DropBufferFull.outcome(),
        Some(Outcome::DroppedBufferFull)
    );
}

#[test]
fn cancel_other_buffers_the_new_action_and_asks_for_the_open_runs_to_be_cancelled() {
    assert_eq!(decision(Overlap::CancelOther, 0, 0), Decision::Start);
    assert_eq!(
        decision(Overlap::CancelOther, 1, 0),
        Decision::Buffer { cancel_open: true }
    );
    assert_eq!(
        decision(Overlap::CancelOther, 3, 0),
        Decision::Buffer { cancel_open: true }
    );
    assert_eq!(
        decision(Overlap::CancelOther, 1, 1),
        Decision::DropBufferFull,
        "D1041: one waiting action, whatever the buffer limit"
    );
}

#[test]
fn allow_all_starts_at_once_until_the_tracking_limit() {
    assert_eq!(decision(Overlap::AllowAll, 0, 0), Decision::Start);
    assert_eq!(decision(Overlap::AllowAll, 99, 0), Decision::Start);
    assert_eq!(
        decision(Overlap::AllowAll, 100, 0),
        Decision::Skip {
            reason: Some(Reason::TrackingLimit)
        },
        "beyond the tracking limit a new action is SKIPPED_OVERLAP with reason TRACKING_LIMIT"
    );
    let raised = Ceilings {
        tracking_limit: 1000,
        ..Ceilings::default()
    };
    assert_eq!(
        decide(&policies(Overlap::AllowAll, 3), &standing(999, 0), &raised),
        Decision::Start
    );
    assert_eq!(
        decide(&policies(Overlap::AllowAll, 3), &standing(1000, 0), &raised),
        Decision::Skip {
            reason: Some(Reason::TrackingLimit)
        }
    );
}

#[test]
fn remaining_actions_at_zero_leave_nothing_to_do_for_every_policy() {
    for overlap in [
        Overlap::Skip,
        Overlap::BufferOne,
        Overlap::BufferAll,
        Overlap::CancelOther,
        Overlap::AllowAll,
    ] {
        let standing = Standing {
            open_runs: 0,
            buffered: 0,
            remaining: Some(0),
        };
        assert_eq!(
            decide(&policies(overlap, 3), &standing, &Ceilings::default()),
            Decision::Exhausted
        );
    }
    assert_eq!(
        Decision::Exhausted.outcome(),
        None,
        "an exhausted schedule records no action"
    );
}

#[test]
fn a_waiting_action_keeps_one_remaining_action_for_itself() {
    let with = |remaining, buffered, open_runs| Standing {
        open_runs,
        buffered,
        remaining: Some(remaining),
    };
    let ceilings = Ceilings::default();
    let all = policies(Overlap::BufferAll, 3);
    assert_eq!(decide(&all, &with(1, 0, 0), &ceilings), Decision::Start);
    assert_eq!(
        decide(&all, &with(1, 0, 1), &ceilings),
        Decision::Buffer { cancel_open: false }
    );
    assert_eq!(
        decide(&all, &with(1, 1, 1), &ceilings),
        Decision::Exhausted,
        "the one remaining action already belongs to the waiting one (D1042)"
    );
    assert_eq!(
        decide(&all, &with(2, 1, 1), &ceilings),
        Decision::Buffer { cancel_open: false }
    );
    let allow = policies(Overlap::AllowAll, 3);
    assert_eq!(
        decide(&allow, &with(2, 0, 5), &ceilings),
        Decision::Start,
        "ALLOW_ALL never buffers"
    );
}

#[test]
fn a_waiting_action_is_released_when_nothing_is_open() {
    assert_eq!(release(&standing(0, 1)), Release::StartOldest);
    assert_eq!(release(&standing(0, 5)), Release::StartOldest);
    assert_eq!(release(&standing(1, 1)), Release::Wait);
    assert_eq!(release(&standing(0, 0)), Release::Nothing);
    assert_eq!(release(&standing(2, 0)), Release::Nothing);
    let exhausted = Standing {
        open_runs: 0,
        buffered: 1,
        remaining: Some(0),
    };
    assert_eq!(
        release(&exhausted),
        Release::StartOldest,
        "its action was reserved when it was buffered"
    );
}

#[test]
fn only_a_failed_ending_pauses_a_schedule_that_asks_for_it() {
    for status in [
        RunStatus::Failed,
        RunStatus::TimedOut,
        RunStatus::CompensationFailed,
        RunStatus::Partial,
    ] {
        assert!(failure_pauses(status), "{status:?}");
    }
    for status in [
        RunStatus::Succeeded,
        RunStatus::Cancelled,
        RunStatus::Compensated,
        RunStatus::Running,
        RunStatus::Waiting,
        RunStatus::Paused,
        RunStatus::Cancelling,
        RunStatus::Compensating,
        RunStatus::ManualReview,
    ] {
        assert!(
            !failure_pauses(status),
            "{status:?}: not CANCELLED, and not before the end"
        );
    }
}

// --- catch-up ---------------------------------------------------------------------------------

fn catch_up_policies(mode: CatchupMode, window_seconds: i64) -> Policies {
    Policies::new(
        Overlap::Skip,
        u64::try_from(window_seconds).expect("window") * 1000,
        mode,
        false,
        100,
        &Ceilings::default(),
    )
    .expect("policies")
}

fn summary(count: u32, first: i64, last: i64) -> Summary {
    Summary {
        count,
        first: at(first),
        last: at(last),
    }
}

#[test]
fn a_time_that_is_on_time_is_simply_acted_on() {
    let now = at(10 * HOUR);
    let plan = plan_catch_up(
        &[at(10 * HOUR)],
        now,
        &catch_up_policies(CatchupMode::One, DAY),
        &Ceilings::default(),
    );
    assert_eq!(
        plan,
        CatchUp {
            missed: None,
            act: vec![at(10 * HOUR)],
            coalesced: None
        }
    );
    assert_eq!(
        plan_catch_up(&[], now, &Policies::default(), &Ceilings::default()),
        CatchUp::default(),
        "nothing due, nothing to plan"
    );
}

#[test]
fn a_three_hour_outage_in_mode_one_acts_on_the_earliest_and_coalesces_the_others() {
    let due = [at(8 * HOUR), at(9 * HOUR), at(10 * HOUR)];
    let plan = plan_catch_up(
        &due,
        at(10 * HOUR + 5),
        &catch_up_policies(CatchupMode::One, DAY),
        &Ceilings::default(),
    );
    assert_eq!(plan.missed, None);
    assert_eq!(plan.act, vec![at(8 * HOUR)]);
    assert_eq!(plan.coalesced, Some(summary(2, 9 * HOUR, 10 * HOUR)));
}

#[test]
fn a_three_hour_outage_in_mode_all_acts_on_each_in_order() {
    let due = [at(8 * HOUR), at(9 * HOUR), at(10 * HOUR)];
    let plan = plan_catch_up(
        &due,
        at(10 * HOUR + 5),
        &catch_up_policies(CatchupMode::All, DAY),
        &Ceilings::default(),
    );
    assert_eq!(plan.act, due.to_vec());
    assert_eq!((plan.missed, plan.coalesced), (None, None));
}

#[test]
fn mode_all_stops_at_the_per_recovery_ceiling_and_coalesces_the_rest() {
    let due: Vec<_> = (1..=150).map(|n| at(n * 60)).collect();
    let now = at(151 * 60);
    let plan = plan_catch_up(
        &due,
        now,
        &catch_up_policies(CatchupMode::All, DAY),
        &Ceilings::default(),
    );
    assert_eq!(plan.act.len(), 100, "the default ceiling is 100");
    assert_eq!(plan.act, due[..100].to_vec());
    assert_eq!(plan.coalesced, Some(summary(50, 101 * 60, 150 * 60)));
    let raised = Ceilings {
        catchup_actions: 1000,
        ..Ceilings::default()
    };
    assert_eq!(
        plan_catch_up(
            &due,
            now,
            &catch_up_policies(CatchupMode::All, DAY),
            &raised
        )
        .act
        .len(),
        150
    );
}

#[test]
fn times_older_than_the_window_form_one_summary_and_the_rest_follow_the_mode() {
    // hourly times; the window is 2 hours; now is 10:00 so 08:00 is exactly on the edge (inside)
    let due: Vec<_> = (1..=10).map(|h| at(h * HOUR)).collect();
    let now = at(10 * HOUR);
    let one = plan_catch_up(
        &due,
        now,
        &catch_up_policies(CatchupMode::One, 2 * HOUR),
        &Ceilings::default(),
    );
    assert_eq!(
        one.missed,
        Some(summary(7, HOUR, 7 * HOUR)),
        "01:00..07:00 are older than 2 hours"
    );
    assert_eq!(
        one.act,
        vec![at(8 * HOUR)],
        "08:00 is exactly two hours old: inside"
    );
    assert_eq!(one.coalesced, Some(summary(2, 9 * HOUR, 10 * HOUR)));
    let all = plan_catch_up(
        &due,
        now,
        &catch_up_policies(CatchupMode::All, 2 * HOUR),
        &Ceilings::default(),
    );
    assert_eq!(all.missed, one.missed);
    assert_eq!(all.act, vec![at(8 * HOUR), at(9 * HOUR), at(10 * HOUR)]);
    assert_eq!(all.coalesced, None);
}

#[test]
fn a_single_time_older_than_the_window_is_only_a_missed_summary() {
    let plan = plan_catch_up(
        &[at(HOUR)],
        at(10 * HOUR),
        &catch_up_policies(CatchupMode::One, HOUR),
        &Ceilings::default(),
    );
    assert_eq!(
        plan,
        CatchUp {
            missed: Some(summary(1, HOUR, HOUR)),
            act: vec![],
            coalesced: None
        }
    );
}

// --- jitter -----------------------------------------------------------------------------------

#[test]
fn the_jitter_seed_is_the_schedule_id_then_the_nominal_time_in_milliseconds() {
    let id = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
    let seed = jitter_seed(id, at(1_000));
    assert_eq!(seed.len(), 24);
    assert_eq!(&seed[..16], id.as_bytes());
    assert_eq!(&seed[16..], &1_000_000_i64.to_be_bytes());
    assert_eq!(
        jitter_seed(id, at(1_000)),
        seed,
        "a function of its arguments only"
    );
    assert_ne!(jitter_seed(id, at(1_001)), seed);
    assert_ne!(jitter_seed(Uuid::from_u128(1), at(1_000)), seed);
    let sub_ms = at(1_000) + time::Duration::microseconds(999);
    assert_eq!(
        jitter_seed(id, sub_ms),
        seed,
        "the key of an action is in whole milliseconds"
    );
}

fn digest(first: u64) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[..8].copy_from_slice(&first.to_be_bytes());
    out[8..].fill(0xAB);
    out
}

#[test]
fn the_jitter_offset_is_the_digest_modulo_the_setting() {
    assert_eq!(
        jitter_offset_ms(&digest(1_234_567), 60_000, None),
        1_234_567 % 60_000
    );
    assert_eq!(
        jitter_offset_ms(&digest(u64::MAX), 86_400_000, None),
        u64::MAX % 86_400_000
    );
    assert_eq!(
        jitter_offset_ms(&digest(99), 0, None),
        0,
        "no jitter configured"
    );
}

#[test]
fn the_jitter_never_reaches_the_next_nominal_time() {
    // the setting is 1 hour but the next time is 5 minutes away
    let offset = jitter_offset_ms(&digest(u64::MAX), 3_600_000, Some(300_000));
    assert!(offset < 300_000, "{offset}");
    assert_eq!(offset, u64::MAX % 300_000);
    assert_eq!(
        jitter_offset_ms(&digest(7), 3_600_000, Some(0)),
        0,
        "no gap, no room"
    );
    assert_eq!(
        jitter_offset_ms(&digest(7), 1_000, Some(3_600_000)),
        7,
        "the setting binds when it is smaller"
    );
}

#[test]
fn nothing_acts_at_or_after_the_end_even_with_jitter() {
    let nominal = at(1_000);
    assert_eq!(
        actual_time(nominal, 500, None),
        Some(at(1_000) + time::Duration::milliseconds(500))
    );
    assert_eq!(
        actual_time(nominal, 500, Some(at(1_001))),
        Some(at(1_000) + time::Duration::milliseconds(500))
    );
    assert_eq!(
        actual_time(nominal, 1_000, Some(at(1_001))),
        None,
        "exclusive: at the end is too late"
    );
    assert_eq!(actual_time(nominal, 5_000, Some(at(1_001))), None);
}
