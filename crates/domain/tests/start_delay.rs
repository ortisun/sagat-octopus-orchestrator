//! A run started with a delay (`T018-006`, `T018-037`, `FR-018-023`, `SC-018-007`): created
//! `WAITING` with its entry held, it releases the entry once at the instant, can be cancelled or
//! time out while it waits without any step having run, and its journal alone rebuilds it.
#![allow(clippy::expect_used)]

mod common;

use std::time::Duration;

use common::{T0, activity, build, id, sid};
use sagat_domain::execution::{
    self, ExecutionError, RunEvent, RunId, RunState, RunStatus, StepStatus, StopReason,
};

const DAY: i64 = 86_400_000_000;

fn until() -> i64 {
    i64::try_from(T0.unix_timestamp_nanos() / 1_000).expect("an instant") + 2 * DAY
}

fn names(events: &[RunEvent]) -> Vec<&'static str> {
    events.iter().map(RunEvent::name).collect()
}

fn delayed() -> (
    sagat_domain::workflow::WorkflowDefinition,
    RunState,
    Vec<RunEvent>,
) {
    let def = build("a", vec![activity("a").with_next(id("b")), activity("b")]);
    let run = RunId::generate();
    let (state, progress) =
        execution::start_run_delayed(&def, run, T0, until()).expect("a delayed start");
    (def, state, progress.events)
}

#[test]
fn a_delayed_run_is_waiting_with_its_entry_held_and_its_journal_rebuilds_it() {
    let (_, state, events) = delayed();

    assert_eq!(names(&events), ["run.started", "run.start_delayed"]);
    assert_eq!(state.status(), RunStatus::Waiting);
    assert!(state.is_start_delayed());
    assert_eq!(state.start_delayed_until_us(), Some(until()));
    assert_eq!(
        state.step(&sid("a")).expect("the entry").status(),
        StepStatus::Pending,
        "the entry is held, not released"
    );
    assert_eq!(
        RunState::replay(state.run_id(), &events).expect("replays"),
        state
    );
}

#[test]
fn nothing_can_be_leased_while_the_entry_is_held() {
    let (_, mut state, _) = delayed();

    let refused = execution::lease_step(&mut state, &sid("a"));

    assert!(refused.is_err(), "{refused:?}");
}

#[test]
fn the_release_lets_the_entry_go_as_a_start_would_have() {
    let (def, mut state, mut events) = delayed();

    let released = execution::release_start(&def, &mut state, T0 + Duration::from_secs(2 * 86_400))
        .expect("the release");
    events.extend(released.events.iter().cloned());

    assert_eq!(
        names(&released.events),
        ["run.start_released", "step.released"]
    );
    assert!(!state.is_start_delayed());
    assert_eq!(state.status(), RunStatus::Running);
    assert_eq!(
        state.step(&sid("a")).expect("the entry").status(),
        StepStatus::Ready
    );
    execution::lease_step(&mut state, &sid("a")).expect("a worker can lease it now");
    assert_eq!(
        RunState::replay(state.run_id(), &events[..3]).expect("replays the prefix"),
        {
            let mut prefix = RunState::new(state.run_id());
            for event in &events[..3] {
                prefix.apply(event).expect("applies");
            }
            prefix
        }
    );
}

#[test]
fn the_entry_is_released_once() {
    let (def, mut state, _) = delayed();
    execution::release_start(&def, &mut state, T0).expect("the release");

    let again = execution::release_start(&def, &mut state, T0);

    assert!(again.is_err(), "{again:?}");
}

#[test]
fn a_run_cancelled_while_it_waits_ends_cancelled_without_a_step_having_run() {
    let (def, mut state, mut events) = delayed();

    let progress = execution::stop_run(&def, &mut state, StopReason::Cancelled, None, T0)
        .expect("the cancellation");
    events.extend(progress.events.iter().cloned());

    assert_eq!(state.status(), RunStatus::Cancelled);
    assert!(
        events
            .iter()
            .all(|event| !matches!(event, RunEvent::StepLeased { .. })),
        "no step ran: {:?}",
        names(&events)
    );
    assert_eq!(
        RunState::replay(state.run_id(), &events).expect("replays"),
        state
    );
}

#[test]
fn a_run_that_passes_its_deadline_while_it_waits_ends_timed_out() {
    let (def, mut state, _) = delayed();

    execution::stop_run(&def, &mut state, StopReason::TimedOut, None, T0).expect("the stop");

    assert_eq!(state.status(), RunStatus::TimedOut);
}

#[test]
fn a_delay_is_recorded_once_and_only_right_after_the_start() {
    let (_, state, events) = delayed();
    let mut again = state.clone();
    assert!(
        again
            .apply(&RunEvent::StartDelayed { until_us: until() })
            .is_err(),
        "a second delay"
    );

    let mut late = RunState::new(RunId::generate());
    late.apply(&events[0]).expect("the start");
    late.apply(&RunEvent::RunPaused).expect("paused");
    assert!(
        late.apply(&RunEvent::StartDelayed { until_us: until() })
            .is_err(),
        "a delay after something else happened"
    );

    let mut fresh = RunState::new(RunId::generate());
    assert!(
        fresh
            .apply(&RunEvent::StartDelayed { until_us: until() })
            .is_err(),
        "a delay before the start"
    );
}

#[test]
fn only_the_recorded_instant_is_released_and_only_a_delayed_run() {
    let (_, state, _) = delayed();
    let mut wrong = state.clone();
    assert!(
        matches!(
            wrong.apply(&RunEvent::StartReleased {
                until_us: until() + 1
            }),
            Err(ExecutionError::Invariant(_))
        ),
        "another instant than the one recorded"
    );

    let (def, _, _) = delayed();
    let (mut undelayed, _) =
        execution::start_run(&def, RunId::generate(), T0).expect("an ordinary start");
    assert!(
        undelayed
            .apply(&RunEvent::StartReleased { until_us: until() })
            .is_err(),
        "a run that was never delayed"
    );
}
