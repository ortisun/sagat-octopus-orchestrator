//! Batch operations of feature 018: the selector, the action set, the job and item state machines
//! and the durable rate slot (`FR-018-027..034`, `T018-005`).
#![allow(clippy::expect_used, clippy::panic)]

use proptest::prelude::*;
use sagat_domain::batch::{
    Action, BatchError, Counters, DeferCause, FailureReason, Filter, ItemState, JobEvent, JobState,
    Justification, Limits, ParamsGiven, Rate, Retry, Selector, SkipReason, next_slot, retry,
};
use sagat_domain::execution::RunStatus;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

fn at(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(seconds).expect("instant")
}

// --- actions and their parameters -------------------------------------------------------------

#[test]
fn the_actions_have_the_names_of_the_contract_and_there_is_no_terminate() {
    let all = [
        (Action::Cancel, "CANCEL"),
        (Action::Signal, "SIGNAL"),
        (Action::Update, "UPDATE"),
        (Action::Rerun, "RERUN"),
        (Action::RetryStep, "RETRY_STEP"),
        (Action::Pause, "PAUSE"),
        (Action::Resume, "RESUME"),
        (Action::MarkManualReview, "MARK_MANUAL_REVIEW"),
        (Action::Repin, "REPIN"),
    ];
    for (action, name) in all {
        assert_eq!(action.as_str(), name);
        assert_eq!(name.parse::<Action>(), Ok(action));
    }
    assert!("TERMINATE".parse::<Action>().is_err());
    assert!(Action::Repin.needs_feature_019());
    assert!(all.iter().filter(|(a, _)| a.needs_feature_019()).count() == 1);
}

#[test]
fn each_action_needs_exactly_its_parameters() {
    let none = ParamsGiven::default();
    let step = ParamsGiven {
        step_id: true,
        ..none
    };
    let signal = ParamsGiven {
        signal: true,
        ..none
    };
    let update = ParamsGiven {
        update: true,
        ..none
    };
    let repin = ParamsGiven {
        repin: true,
        ..none
    };
    let table: [(Action, ParamsGiven); 9] = [
        (Action::Cancel, none),
        (Action::Pause, none),
        (Action::Resume, none),
        (Action::Rerun, none),
        (Action::RetryStep, step),
        (Action::MarkManualReview, step),
        (Action::Signal, signal),
        (Action::Update, update),
        (Action::Repin, repin),
    ];
    let everything = [
        none,
        step,
        signal,
        update,
        repin,
        ParamsGiven {
            step_id: true,
            signal: true,
            ..none
        },
    ];
    for (action, right) in table {
        assert_eq!(
            action.check_params(right),
            Ok(()),
            "{action:?} with its own parameters"
        );
        for given in everything {
            if given != right {
                assert_eq!(
                    action.check_params(given),
                    Err(BatchError::ActionParameters),
                    "{action:?} with {given:?} needs or refuses a parameter"
                );
            }
        }
    }
}

#[test]
fn the_reason_is_eight_to_two_thousand_characters() {
    assert!(Justification::new(&"é".repeat(8)).is_ok());
    assert!(Justification::new(&"é".repeat(2000)).is_ok());
    assert_eq!(
        Justification::new(&"é".repeat(7)).expect_err("short"),
        BatchError::BadReason
    );
    assert_eq!(
        Justification::new(&"é".repeat(2001)).expect_err("long"),
        BatchError::BadReason
    );
    assert_eq!(
        Justification::new("clean up").expect("reason").as_str(),
        "clean up"
    );
}

// --- selection --------------------------------------------------------------------------------

#[test]
fn an_explicit_list_is_one_to_a_thousand_distinct_run_ids() {
    let ids = |n: usize| (1..=n as u128).map(Uuid::from_u128).collect::<Vec<_>>();
    assert!(Selector::run_ids(ids(1)).is_ok());
    assert!(Selector::run_ids(ids(1000)).is_ok());
    assert_eq!(
        Selector::run_ids(vec![]).expect_err("empty"),
        BatchError::SelectorInvalid
    );
    assert_eq!(
        Selector::run_ids(ids(1001)).expect_err("1001"),
        BatchError::SelectorInvalid
    );
    let twice = vec![Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(1)];
    assert_eq!(
        Selector::run_ids(twice).expect_err("duplicate"),
        BatchError::SelectorInvalid,
        "uniqueItems"
    );
}

#[test]
fn a_filter_keeps_its_ranges_in_order_and_its_lists_bounded() {
    let ok = Filter {
        statuses: vec![RunStatus::Failed, RunStatus::TimedOut],
        created_after: Some(at(10)),
        created_before: Some(at(20)),
        business_id_prefix: Some("order-".to_owned()),
        ..Filter::default()
    };
    assert!(Selector::filter(ok).is_ok());
    assert!(
        Selector::filter(Filter::default()).is_ok(),
        "the contract requires no field"
    );

    let backwards = Filter {
        created_after: Some(at(20)),
        created_before: Some(at(10)),
        ..Filter::default()
    };
    assert_eq!(
        Selector::filter(backwards).expect_err("backwards"),
        BatchError::SelectorInvalid
    );
    let empty_range = Filter {
        created_after: Some(at(10)),
        created_before: Some(at(10)),
        ..Filter::default()
    };
    assert_eq!(
        Selector::filter(empty_range).expect_err("empty range"),
        BatchError::SelectorInvalid
    );

    let repeated = Filter {
        statuses: vec![RunStatus::Failed, RunStatus::Failed],
        ..Filter::default()
    };
    assert_eq!(
        Selector::filter(repeated).expect_err("repeated status"),
        BatchError::SelectorInvalid
    );

    for bad in ["", "has space", "caf\u{e9}", &"x".repeat(201)] {
        let filter = Filter {
            business_id_prefix: Some(bad.to_owned()),
            ..Filter::default()
        };
        assert_eq!(
            Selector::filter(filter).expect_err(bad),
            BatchError::SelectorInvalid,
            "{bad:?}"
        );
    }
}

#[test]
fn the_limits_default_to_the_plan_and_are_clamped_to_the_hard_ceilings() {
    let d = Limits::default();
    assert_eq!(
        (d.max_items, d.rate),
        (10_000, Rate::per_second(20).expect("rate"))
    );
    assert_eq!((d.item_retries, d.retry_window), (10, Duration::hours(1)));
    let clamped = Limits {
        max_items: 10_000_000,
        item_retries: 5000,
        retry_window: Duration::days(30),
        ..Limits::default()
    }
    .clamped();
    assert_eq!(clamped.max_items, 100_000);
    assert_eq!(clamped.item_retries, 50);
    assert_eq!(clamped.retry_window, Duration::hours(24));
    assert_eq!(
        Limits::new(0, Rate::default()).expect_err("zero items"),
        BatchError::BadLimits
    );
    assert_eq!(
        Limits::new(100_001, Rate::default()).expect_err("too many"),
        BatchError::BadLimits
    );
    assert!(Limits::new(100_000, Rate::default()).is_ok());
}

// --- rate slots -------------------------------------------------------------------------------

#[test]
fn a_rate_is_a_thousandth_of_an_item_per_second_up_to_two_hundred() {
    assert!(Rate::from_milli(1).is_ok());
    assert!(Rate::from_milli(200_000).is_ok());
    assert_eq!(Rate::from_milli(0).expect_err("zero"), BatchError::BadRate);
    assert_eq!(
        Rate::from_milli(200_001).expect_err("fast"),
        BatchError::BadRate
    );
    assert_eq!(
        Rate::per_second(201).expect_err("fast"),
        BatchError::BadRate
    );
    assert_eq!(Rate::default().milli(), 20_000);
    assert_eq!(
        Rate::per_second(20).expect("rate").interval(),
        Duration::milliseconds(50)
    );
    assert_eq!(
        Rate::per_second(200).expect("rate").interval(),
        Duration::milliseconds(5)
    );
    assert_eq!(
        Rate::from_milli(500).expect("rate").interval(),
        Duration::seconds(2),
        "half an item a second"
    );
    assert_eq!(
        Rate::from_milli(1).expect("rate").interval(),
        Duration::seconds(1000)
    );
}

#[test]
fn the_next_slot_never_lets_a_late_item_cause_a_burst() {
    let rate = Rate::per_second(10).expect("rate"); // one every 100 ms
    let slot = at(1_000);
    // applied on time: the next slot is one interval after the slot it used
    assert_eq!(
        next_slot(slot, slot, rate),
        slot + Duration::milliseconds(100)
    );
    // applied late: the schedule restarts from now, so two items never run closer than an interval
    let late = slot + Duration::seconds(30);
    assert_eq!(
        next_slot(slot, late, rate),
        late + Duration::milliseconds(100)
    );
    // applied before the slot (a retry that raced it): the slot still moves forward from the slot
    assert_eq!(
        next_slot(slot, slot - Duration::seconds(1), rate),
        slot + Duration::milliseconds(100)
    );
}

proptest! {
    /// However late the items are applied, two applications are never closer than one interval.
    #[test]
    fn items_are_never_closer_than_the_interval(
        milli in 1..=200_000u32,
        delays in proptest::collection::vec(prop_oneof![Just(0i64), 0..50i64, 0..5_000_000i64], 1..40),
    ) {
        let rate = Rate::from_milli(milli).expect("rate");
        let interval = rate.interval();
        let mut slot = at(0);
        let mut now = at(0);
        let mut last_applied: Option<OffsetDateTime> = None;
        for delay in delays {
            now = now.max(slot) + Duration::milliseconds(delay);
            if let Some(last) = last_applied {
                prop_assert!(now - last >= interval, "{:?} after {:?} with interval {:?}", now, last, interval);
            }
            last_applied = Some(now);
            slot = next_slot(slot, now, rate);
        }
    }
}

// --- the job ----------------------------------------------------------------------------------

#[test]
fn a_job_moves_through_the_diagram_of_the_data_model_and_nowhere_else() {
    use JobEvent::{CancelDrained, CancelRequested, Failed, ItemsDone, SelectionFinished};
    use JobState::{Cancelled, Cancelling, Completed, Failed as Fail, Running, Selecting};
    let authority = Failed(FailureReason::AuthorityRevoked);
    let table: [(JobState, JobEvent, Option<JobState>); 24] = [
        (Selecting, SelectionFinished, Some(Running)),
        (Selecting, CancelRequested, Some(Cancelling)),
        (Selecting, authority, Some(Fail)),
        (Selecting, ItemsDone, None),
        (Selecting, CancelDrained, None),
        (Running, ItemsDone, Some(Completed)),
        (Running, CancelRequested, Some(Cancelling)),
        (Running, authority, Some(Fail)),
        (Running, SelectionFinished, None),
        (Running, CancelDrained, None),
        (Cancelling, CancelDrained, Some(Cancelled)),
        (Cancelling, authority, None),
        (Cancelling, ItemsDone, None),
        (Cancelling, CancelRequested, None),
        (Cancelling, SelectionFinished, None),
        (Completed, CancelRequested, None),
        (Completed, ItemsDone, None),
        (Completed, authority, None),
        (Cancelled, CancelRequested, None),
        (Cancelled, CancelDrained, None),
        (Fail, CancelRequested, None),
        (Fail, authority, None),
        (Fail, SelectionFinished, None),
        (Fail, ItemsDone, None),
    ];
    for (from, event, expected) in table {
        match expected {
            Some(to) => assert_eq!(from.on(event), Ok(to), "{from:?} --{event:?}-->"),
            None => assert_eq!(
                from.on(event),
                Err(BatchError::IllegalTransition),
                "{from:?} --{event:?}--> is not a transition"
            ),
        }
    }
}

#[test]
fn a_job_counts_against_the_active_ceiling_until_it_ends() {
    let states = [
        (JobState::Selecting, true, false),
        (JobState::Running, true, false),
        (JobState::Cancelling, true, false),
        (JobState::Completed, false, true),
        (JobState::Cancelled, false, true),
        (JobState::Failed, false, true),
    ];
    for (state, active, terminal) in states {
        assert_eq!(
            (state.is_active(), state.is_terminal()),
            (active, terminal),
            "{state:?}"
        );
        assert_eq!(state.as_str().parse::<JobState>(), Ok(state));
    }
    assert_eq!(JobState::Cancelling.as_str(), "CANCELLING");
    assert!(
        JobState::Cancelling.is_active(),
        "an in-flight item may still commit"
    );
    assert!(!JobState::Cancelling.accepts_new_items() && JobState::Running.accepts_new_items());
    assert!(
        !JobState::Selecting.accepts_new_items(),
        "items start after selection"
    );
}

// --- items ------------------------------------------------------------------------------------

#[test]
fn an_item_ends_once_and_may_be_deferred_and_retried_until_then() {
    use ItemState::{Cancelled, Deferred, Failed, Pending, Skipped, Succeeded};
    let ends = [Succeeded, Skipped, Failed, Cancelled];
    for from in [Pending, Deferred] {
        for to in ends {
            assert!(from.can_become(to), "{from:?} -> {to:?}");
        }
        assert!(from.can_become(Deferred), "{from:?} may be deferred again");
    }
    assert!(Pending.can_become(Deferred) && Deferred.can_become(Pending));
    assert!(!Pending.can_become(Pending));
    for from in ends {
        for to in [Pending, Deferred, Succeeded, Skipped, Failed, Cancelled] {
            assert!(!from.can_become(to), "{from:?} is final");
        }
        assert!(from.is_terminal());
    }
    assert!(!Pending.is_terminal() && !Deferred.is_terminal());
}

#[test]
fn the_reasons_and_causes_have_the_names_of_the_contract() {
    let skips = [
        (SkipReason::NotApplicable, "NOT_APPLICABLE"),
        (SkipReason::NotWaiting, "NOT_WAITING"),
        (SkipReason::OperationNotDeclared, "OPERATION_NOT_DECLARED"),
        (SkipReason::RunContinued, "RUN_CONTINUED"),
        (SkipReason::AlreadyTerminal, "ALREADY_TERMINAL"),
        (SkipReason::HiddenFromCreator, "HIDDEN_FROM_CREATOR"),
        (SkipReason::TestRun, "TEST_RUN"),
    ];
    for (reason, name) in skips {
        assert_eq!(reason.as_str(), name);
    }
    let causes = [
        (DeferCause::QuotaExceeded, "QUOTA_EXCEEDED"),
        (DeferCause::RateLimited, "RATE_LIMITED"),
        (DeferCause::ConcurrencyQueueFull, "CONCURRENCY_QUEUE_FULL"),
        (DeferCause::LockWait, "LOCK_WAIT"),
    ];
    for (cause, name) in causes {
        assert_eq!(cause.as_str(), name);
    }
    assert_eq!(
        FailureReason::AuthorityRevoked.as_str(),
        "AUTHORITY_REVOKED"
    );
    assert_eq!(FailureReason::SelectionFailed.as_str(), "SELECTION_FAILED");
}

#[test]
fn a_refused_item_waits_five_seconds_doubling_to_five_minutes() {
    let first = at(0);
    let waits: Vec<i64> = (1..=10)
        .map(
            |attempts| match retry(attempts, first, first, &Limits::default()) {
                Retry::After(d) => d.whole_seconds(),
                Retry::Exhausted => -1,
            },
        )
        .collect();
    assert_eq!(waits, [5, 10, 20, 40, 80, 160, 300, 300, 300, 300]);
}

#[test]
fn an_item_gives_up_after_ten_attempts_or_an_hour() {
    let first = at(0);
    let limits = Limits::default();
    assert_eq!(
        retry(11, first, first, &limits),
        Retry::Exhausted,
        "the eleventh refusal"
    );
    assert!(matches!(retry(10, first, first, &limits), Retry::After(_)));
    let just_inside = first + Duration::seconds(3599);
    let at_an_hour = first + Duration::seconds(3600);
    assert!(matches!(
        retry(1, first, just_inside, &limits),
        Retry::After(_)
    ));
    assert_eq!(
        retry(1, first, at_an_hour, &limits),
        Retry::Exhausted,
        "an hour since the first refusal"
    );
}

// --- counters ---------------------------------------------------------------------------------

#[test]
fn counters_equal_the_outcomes_of_the_items() {
    let mut c = Counters::default();
    c.select(5);
    assert_eq!((c.selected(), c.pending()), (5, 5));
    c.apply(ItemState::Pending, ItemState::Succeeded)
        .expect("1");
    c.apply(ItemState::Pending, ItemState::Deferred).expect("2");
    c.apply(ItemState::Deferred, ItemState::Failed)
        .expect("2 again");
    c.apply(ItemState::Pending, ItemState::Skipped).expect("3");
    c.apply(ItemState::Pending, ItemState::Cancelled)
        .expect("4");
    assert_eq!(
        (
            c.succeeded(),
            c.skipped(),
            c.failed(),
            c.cancelled(),
            c.deferred()
        ),
        (1, 1, 1, 1, 0)
    );
    assert_eq!((c.pending(), c.terminal()), (1, 4));
    assert!(!c.is_drained());
    c.apply(ItemState::Pending, ItemState::Succeeded)
        .expect("5");
    assert!(c.is_drained(), "every selected item has ended");
}

#[test]
fn counters_refuse_a_move_the_items_cannot_have_made() {
    let mut c = Counters::default();
    assert_eq!(
        c.apply(ItemState::Pending, ItemState::Succeeded),
        Err(BatchError::CounterInvariant),
        "nothing selected"
    );
    c.select(1);
    assert_eq!(
        c.apply(ItemState::Deferred, ItemState::Failed),
        Err(BatchError::CounterInvariant),
        "none deferred"
    );
    assert_eq!(
        c.apply(ItemState::Succeeded, ItemState::Failed),
        Err(BatchError::CounterInvariant),
        "final"
    );
    assert_eq!(
        c.apply(ItemState::Pending, ItemState::Pending),
        Err(BatchError::CounterInvariant)
    );
    assert_eq!(
        (c.succeeded(), c.failed(), c.deferred()),
        (0, 0, 0),
        "a refused move changes nothing"
    );
}

proptest! {
    /// Whatever sequence of moves the items make, the counters never exceed what was selected and
    /// always add up.
    #[test]
    fn counters_add_up(moves in proptest::collection::vec((0..6usize, 0..6usize), 0..200), selected in 0..50u32) {
        let states = [
            ItemState::Pending, ItemState::Deferred, ItemState::Succeeded,
            ItemState::Skipped, ItemState::Failed, ItemState::Cancelled,
        ];
        let mut c = Counters::default();
        c.select(selected);
        for (from, to) in moves {
            let _ = c.apply(states[from], states[to]);
            prop_assert_eq!(c.pending() + c.deferred() + c.terminal(), c.selected());
            prop_assert_eq!(c.terminal(), c.succeeded() + c.skipped() + c.failed() + c.cancelled());
        }
    }
}
