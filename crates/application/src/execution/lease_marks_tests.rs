//! The pure side of the marks of protocol 1.8 (feature 006, Task 006-047): the order of the engine's reasons not to mark, the
//! condition under which a mark counts, and the attempt deadline a provisional lease carries. The SQL side is compared with
//! these by the parity and matrix tests of `integration_durable_execution`.

use time::{Duration, OffsetDateTime};

use super::{ClaimMark, LeaseDeadlines, LeaseMarks, MarkOutcome, MarkSkip};

#[test]
fn the_engine_s_reasons_not_to_mark_are_judged_in_the_order_of_the_contract() {
    // every combination of the four facts the engine knows, and the first reason that holds
    for bits in 0u8..16 {
        let activated = bits & 1 != 0;
        let floor = bits & 2 != 0;
        let switch_on = bits & 4 != 0;
        let declines = bits & 8 != 0;
        let expected = if !activated {
            Some(MarkSkip::NotActivated)
        } else if !floor {
            Some(MarkSkip::RequirementMissing)
        } else if !switch_on {
            Some(MarkSkip::SwitchOff)
        } else if declines {
            Some(MarkSkip::Declined)
        } else {
            None
        };
        assert_eq!(
            MarkSkip::of_engine(activated, floor, switch_on, declines),
            expected,
            "activated={activated} floor={floor} switch_on={switch_on} declines={declines}"
        );
    }
}

#[test]
fn every_reason_has_its_label_and_is_read_back_from_it() {
    let labels: Vec<&str> = MarkSkip::ALL.iter().map(|reason| reason.as_str()).collect();
    assert_eq!(
        labels,
        [
            "not_activated",
            "requirement_missing",
            "switch_off",
            "declined",
            "flagged_before",
            "short_deadline"
        ]
    );
    for reason in MarkSkip::ALL {
        assert_eq!(MarkSkip::parse(reason.as_str()), Some(reason));
    }
    assert_eq!(MarkSkip::parse("unmarked"), None);
}

fn at(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::UNIX_EPOCH + Duration::seconds(1_000_000 + seconds)
}

fn marks() -> LeaseMarks {
    LeaseMarks {
        deadlines: LeaseDeadlines {
            expires_at: at(30),
            attempt_deadline_at: None,
        },
        held: true,
        generation: 4,
        hand_back_generation: Some(4),
        claimed_at: Some(at(0)),
        claimed_on_this_server: true,
    }
}

#[test]
fn a_mark_counts_only_for_the_held_lease_of_its_generation_claimed_on_this_server() {
    assert!(marks().counts());
    let cases = [
        (
            "not held",
            LeaseMarks {
                held: false,
                ..marks()
            },
        ),
        (
            "an older generation",
            LeaseMarks {
                hand_back_generation: Some(3),
                ..marks()
            },
        ),
        (
            "no reservation",
            LeaseMarks {
                hand_back_generation: None,
                ..marks()
            },
        ),
        (
            "another server, or none",
            LeaseMarks {
                claimed_on_this_server: false,
                ..marks()
            },
        ),
    ];
    for (case, lease) in cases {
        assert!(!lease.counts(), "{case}");
        assert!(!lease.counts_since(at(-10)), "{case}");
    }
}

#[test]
fn with_the_recording_a_mark_counts_only_for_a_lease_claimed_strictly_after_it() {
    let lease = marks();
    assert!(lease.counts_since(at(-1)));
    assert!(!lease.counts_since(at(0)), "an equal time does not count");
    assert!(!lease.counts_since(at(1)), "claimed before the recording");
    let unclaimed = LeaseMarks {
        claimed_at: None,
        ..marks()
    };
    assert!(
        unclaimed.counts(),
        "the form without the recording reads no time"
    );
    assert!(!unclaimed.counts_since(at(-1)));
}

#[test]
fn a_provisional_lease_carries_its_attempt_deadline_only_when_it_comes_before_the_expiry() {
    let mark = |outcome, attempt_deadline_at| ClaimMark {
        outcome,
        deadlines: LeaseDeadlines {
            expires_at: at(60),
            attempt_deadline_at,
        },
    };
    assert_eq!(
        mark(MarkOutcome::Provisional, Some(at(20))).attempt_deadline(),
        Some(at(20))
    );
    assert_eq!(
        mark(MarkOutcome::Provisional, Some(at(60))).attempt_deadline(),
        None
    );
    assert_eq!(
        mark(MarkOutcome::Provisional, Some(at(90))).attempt_deadline(),
        None
    );
    assert_eq!(
        mark(MarkOutcome::Provisional, None).attempt_deadline(),
        None
    );
    for other in [
        MarkOutcome::Unreserved,
        MarkOutcome::Skipped(MarkSkip::Declined),
    ] {
        assert_eq!(mark(other, Some(at(20))).attempt_deadline(), None);
    }
}
