//! Business identifiers of feature 018: the value, the two policies, and the pure admission that
//! says what a start with an id becomes (`FR-018-018..020`, `T018-004`).
#![allow(clippy::expect_used, clippy::panic)]

use proptest::prelude::*;
use sagat_domain::business_ids::{
    Admission, Binding, BusinessId, BusinessIdError, ConflictPolicy, Held, HolderStatus,
    ReusePolicy, ReuseWindow, admit,
};
use sagat_domain::execution::RunStatus;
use time::OffsetDateTime;

fn at(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(seconds).expect("instant")
}

const DAY: i64 = 86_400;

fn window(days: i64) -> ReuseWindow {
    ReuseWindow::from_seconds(u32::try_from(days * DAY).expect("window")).expect("window")
}

// --- the value --------------------------------------------------------------------------------

#[test]
fn an_id_is_one_to_two_hundred_visible_ascii_characters() {
    for ok in [
        "a",
        "order-123",
        "!~",
        "ORDER/2026:10:29#7",
        &"x".repeat(200),
    ] {
        assert_eq!(BusinessId::new(ok).expect(ok).as_str(), ok);
    }
    assert_eq!(
        BusinessId::new("").expect_err("empty"),
        BusinessIdError::Empty
    );
    assert_eq!(
        BusinessId::new(&"x".repeat(201)).expect_err("long"),
        BusinessIdError::TooLong
    );
    for bad in [
        "has space",
        "tab\there",
        "line\nbreak",
        "caf\u{e9}",
        "del\u{7f}",
        "nul\0",
        " leading",
        "trailing ",
    ] {
        assert_eq!(
            BusinessId::new(bad).expect_err(bad),
            BusinessIdError::NotVisibleAscii,
            "{bad:?}: space is not visible"
        );
    }
}

#[test]
fn the_length_is_counted_in_characters_and_a_long_non_ascii_id_is_refused_for_its_kind() {
    // the length is checked before the kind of character: 201 two-byte characters are too long
    assert_eq!(
        BusinessId::new(&"\u{e9}".repeat(201)).expect_err("long"),
        BusinessIdError::TooLong
    );
    assert_eq!(
        BusinessId::new(&"\u{e9}".repeat(3)).expect_err("short"),
        BusinessIdError::NotVisibleAscii
    );
}

#[test]
fn the_defaults_fail_on_conflict_and_allow_a_duplicate() {
    assert_eq!(ConflictPolicy::default(), ConflictPolicy::Fail);
    assert_eq!(ReusePolicy::default(), ReusePolicy::AllowDuplicate);
    assert_eq!(ReuseWindow::default().seconds(), 30 * 86_400);
    for (policy, name) in [
        (ConflictPolicy::Fail, "FAIL"),
        (ConflictPolicy::UseExisting, "USE_EXISTING"),
    ] {
        assert_eq!(policy.as_str(), name);
        assert_eq!(name.parse::<ConflictPolicy>(), Ok(policy));
    }
    for (policy, name) in [
        (ReusePolicy::AllowDuplicate, "ALLOW_DUPLICATE"),
        (
            ReusePolicy::AllowDuplicateFailedOnly,
            "ALLOW_DUPLICATE_FAILED_ONLY",
        ),
        (ReusePolicy::RejectDuplicate, "REJECT_DUPLICATE"),
    ] {
        assert_eq!(policy.as_str(), name);
        assert_eq!(name.parse::<ReusePolicy>(), Ok(policy));
    }
    assert!(
        "TERMINATE_EXISTING".parse::<ConflictPolicy>().is_err(),
        "there is no terminate-existing"
    );
}

#[test]
fn the_reuse_window_is_a_second_to_a_leap_year() {
    assert!(ReuseWindow::from_seconds(1).is_ok());
    assert!(ReuseWindow::from_seconds(31_622_400).is_ok());
    assert_eq!(
        ReuseWindow::from_seconds(0).expect_err("zero"),
        BusinessIdError::BadReuseWindow
    );
    assert_eq!(
        ReuseWindow::from_seconds(31_622_401).expect_err("over"),
        BusinessIdError::BadReuseWindow
    );
}

// --- the holder -------------------------------------------------------------------------------

#[test]
fn a_holder_is_closed_only_with_a_terminal_status_and_a_finish_time() {
    let finished = at(1_000);
    for status in [
        RunStatus::Succeeded,
        RunStatus::Failed,
        RunStatus::Cancelled,
        RunStatus::TimedOut,
        RunStatus::Compensated,
        RunStatus::Partial,
        RunStatus::CompensationFailed,
    ] {
        assert_eq!(
            HolderStatus::of(status, Some(finished)),
            HolderStatus::Closed {
                status,
                finished_at: finished
            }
        );
    }
    for status in [
        RunStatus::Running,
        RunStatus::Waiting,
        RunStatus::Paused,
        RunStatus::Cancelling,
        RunStatus::Compensating,
        RunStatus::ManualReview,
    ] {
        assert_eq!(
            HolderStatus::of(status, None),
            HolderStatus::Open,
            "{status:?}"
        );
        assert_eq!(
            HolderStatus::of(status, Some(finished)),
            HolderStatus::Open,
            "{status:?}: not terminal"
        );
    }
    assert_eq!(
        HolderStatus::of(RunStatus::Failed, None),
        HolderStatus::Open,
        "a terminal run with no finish time is not trusted to be free: the id stays held"
    );
}

// --- admission --------------------------------------------------------------------------------

fn binding(days: i64) -> Binding {
    Binding::new(window(days))
}

fn closed(status: RunStatus, finished_at: i64) -> HolderStatus {
    HolderStatus::Closed {
        status,
        finished_at: at(finished_at),
    }
}

#[test]
fn a_free_id_is_bound_whatever_the_policies() {
    for conflict in [ConflictPolicy::Fail, ConflictPolicy::UseExisting] {
        for reuse in [
            ReusePolicy::AllowDuplicate,
            ReusePolicy::AllowDuplicateFailedOnly,
            ReusePolicy::RejectDuplicate,
        ] {
            assert_eq!(admit(None, conflict, reuse, at(0)), Admission::Bind);
        }
    }
}

#[test]
fn an_open_holder_fails_the_start_or_is_returned() {
    let held = Held {
        binding: binding(30),
        holder: HolderStatus::Open,
    };
    for reuse in [
        ReusePolicy::AllowDuplicate,
        ReusePolicy::AllowDuplicateFailedOnly,
        ReusePolicy::RejectDuplicate,
    ] {
        assert_eq!(
            admit(Some(&held), ConflictPolicy::Fail, reuse, at(0)),
            Admission::InUse
        );
        assert_eq!(
            admit(Some(&held), ConflictPolicy::UseExisting, reuse, at(0)),
            Admission::UseExisting,
            "the reuse policy is about closed holders only"
        );
    }
    assert!(!Admission::InUse.creates_a_run() && !Admission::UseExisting.creates_a_run());
    assert!(Admission::Bind.creates_a_run());
    assert!(!Admission::ReuseRejected.creates_a_run());
}

#[test]
fn a_closed_holder_inside_the_window_follows_the_reuse_policy() {
    // finished a day ago, window 30 days
    let now = at(10 * DAY);
    let table: [(RunStatus, ReusePolicy, Admission); 12] = [
        (
            RunStatus::Succeeded,
            ReusePolicy::AllowDuplicate,
            Admission::Bind,
        ),
        (
            RunStatus::Succeeded,
            ReusePolicy::AllowDuplicateFailedOnly,
            Admission::ReuseRejected,
        ),
        (
            RunStatus::Succeeded,
            ReusePolicy::RejectDuplicate,
            Admission::ReuseRejected,
        ),
        (
            RunStatus::Failed,
            ReusePolicy::AllowDuplicate,
            Admission::Bind,
        ),
        (
            RunStatus::Failed,
            ReusePolicy::AllowDuplicateFailedOnly,
            Admission::Bind,
        ),
        (
            RunStatus::Failed,
            ReusePolicy::RejectDuplicate,
            Admission::ReuseRejected,
        ),
        (
            RunStatus::Compensated,
            ReusePolicy::AllowDuplicateFailedOnly,
            Admission::Bind,
        ),
        (
            RunStatus::Partial,
            ReusePolicy::AllowDuplicateFailedOnly,
            Admission::Bind,
        ),
        (
            RunStatus::Cancelled,
            ReusePolicy::AllowDuplicateFailedOnly,
            Admission::Bind,
        ),
        (
            RunStatus::TimedOut,
            ReusePolicy::AllowDuplicateFailedOnly,
            Admission::Bind,
        ),
        (
            RunStatus::CompensationFailed,
            ReusePolicy::AllowDuplicateFailedOnly,
            Admission::Bind,
        ),
        (
            RunStatus::Compensated,
            ReusePolicy::RejectDuplicate,
            Admission::ReuseRejected,
        ),
    ];
    for (status, reuse, expected) in table {
        let held = Held {
            binding: binding(30),
            holder: closed(status, 9 * DAY),
        };
        for conflict in [ConflictPolicy::Fail, ConflictPolicy::UseExisting] {
            assert_eq!(
                admit(Some(&held), conflict, reuse, now),
                expected,
                "{status:?} {reuse:?} {conflict:?}"
            );
        }
    }
}

#[test]
fn the_id_is_free_from_the_instant_the_window_ends() {
    let held = Held {
        binding: binding(30),
        holder: closed(RunStatus::Succeeded, 0),
    };
    let end = 30 * DAY;
    assert_eq!(
        admit(
            Some(&held),
            ConflictPolicy::Fail,
            ReusePolicy::RejectDuplicate,
            at(end - 1)
        ),
        Admission::ReuseRejected,
        "one second before the end"
    );
    for now in [end, end + 1, end + 365 * DAY] {
        assert_eq!(
            admit(
                Some(&held),
                ConflictPolicy::Fail,
                ReusePolicy::RejectDuplicate,
                at(now)
            ),
            Admission::Bind,
            "after the window the id is free, whatever the policy"
        );
    }
}

#[test]
fn the_window_is_the_one_frozen_at_binding_not_the_current_setting() {
    let short = Held {
        binding: binding(1),
        holder: closed(RunStatus::Succeeded, 0),
    };
    let long = Held {
        binding: binding(300),
        holder: closed(RunStatus::Succeeded, 0),
    };
    let now = at(2 * DAY);
    assert_eq!(
        admit(
            Some(&short),
            ConflictPolicy::Fail,
            ReusePolicy::RejectDuplicate,
            now
        ),
        Admission::Bind
    );
    assert_eq!(
        admit(
            Some(&long),
            ConflictPolicy::Fail,
            ReusePolicy::RejectDuplicate,
            now
        ),
        Admission::ReuseRejected
    );
}

#[test]
fn a_finish_time_in_the_future_keeps_the_id_held() {
    // clock skew between processes must not free an id early; the window runs from the finish time
    let held = Held {
        binding: binding(30),
        holder: closed(RunStatus::Succeeded, 100 * DAY),
    };
    assert_eq!(
        admit(
            Some(&held),
            ConflictPolicy::Fail,
            ReusePolicy::RejectDuplicate,
            at(0)
        ),
        Admission::ReuseRejected
    );
}

proptest! {
    /// An open holder is never replaced, whatever the reuse policy, window or clock.
    #[test]
    fn an_open_holder_is_never_replaced(
        conflict in prop_oneof![Just(ConflictPolicy::Fail), Just(ConflictPolicy::UseExisting)],
        reuse in prop_oneof![
            Just(ReusePolicy::AllowDuplicate),
            Just(ReusePolicy::AllowDuplicateFailedOnly),
            Just(ReusePolicy::RejectDuplicate),
        ],
        days in 1..=366i64,
        now in -1_000_000_000..4_000_000_000i64,
    ) {
        let held = Held { binding: binding(days), holder: HolderStatus::Open };
        let admission = admit(Some(&held), conflict, reuse, at(now));
        prop_assert!(matches!(admission, Admission::InUse | Admission::UseExisting));
        prop_assert!(!admission.creates_a_run());
    }

    /// Past its window a closed holder never blocks a start; inside it only the reuse policy can.
    #[test]
    fn a_closed_holder_blocks_only_inside_its_window_and_only_by_policy(
        reuse in prop_oneof![
            Just(ReusePolicy::AllowDuplicate),
            Just(ReusePolicy::AllowDuplicateFailedOnly),
            Just(ReusePolicy::RejectDuplicate),
        ],
        succeeded in any::<bool>(),
        days in 1..=366i64,
        finished in 0..2_000_000_000i64,
        elapsed in 0..40_000_000i64,
    ) {
        let status = if succeeded { RunStatus::Succeeded } else { RunStatus::Failed };
        let held = Held { binding: binding(days), holder: closed(status, finished) };
        let admission = admit(Some(&held), ConflictPolicy::Fail, reuse, at(finished + elapsed));
        let inside = elapsed < days * DAY;
        if !inside || reuse == ReusePolicy::AllowDuplicate {
            prop_assert_eq!(admission, Admission::Bind);
        } else {
            let refused = reuse == ReusePolicy::RejectDuplicate || succeeded;
            prop_assert_eq!(admission, if refused { Admission::ReuseRejected } else { Admission::Bind });
        }
    }
}
